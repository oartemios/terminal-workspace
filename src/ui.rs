use crate::{
    Action, App, Block, Command, CommandInvocation, CommandOutcome, Group, GroupView, Item,
    Navigation,
};
use console::{truncate_str, Key};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    Plugins,
    Groups,
    Items,
}

impl Focus {
    fn next(self) -> Self {
        match self {
            Self::Plugins => Self::Groups,
            Self::Groups => Self::Items,
            Self::Items => Self::Plugins,
        }
    }
    fn previous(self) -> Self {
        match self {
            Self::Plugins => Self::Items,
            Self::Groups => Self::Plugins,
            Self::Items => Self::Groups,
        }
    }
}

enum Mode {
    Normal,
    Actions {
        entries: Vec<Action>,
        selected: usize,
    },
    Palette {
        query: String,
        selected: usize,
    },
    Command(String),
    Search {
        previous: String,
    },
    Output {
        block: Block,
        offset: usize,
    },
    Help {
        offset: usize,
    },
}

#[derive(Clone, Copy)]
enum Sort {
    Plugin,
    Ascending,
    Descending,
}

// Prototype defaults, not part of the Plugin API or product requirements.
const BINDINGS: &[(&str, &str)] = &[
    ("fp", "files.preview"),
    ("fs", "files.path"),
    ("fo", "files.open"),
    ("fu", "files.parent"),
];
const HELP: &[&str] = &[
    "Tab / Shift-Tab   focus plugins, groups, items",
    "Arrows or h j k l   navigate; Enter advances",
    "Enter / l   default action; a   all actions",
    "Backspace / h   parent context (when available)",
    "Space   searchable command palette",
    ":   command line; selected item is default context",
    "/   filter items; Esc clears the filter",
    "s   sort: plugin order / title ascending / descending",
    "r   refresh the current group",
    "f p   preview; f s   path; f o   open; f u   parent",
    "PgUp/PgDn, Home/End   scroll list or output",
    "Esc   back; q / Ctrl-C / Ctrl-D   quit",
];

struct ListState {
    selected: Option<String>,
    filter: String,
    sort: Sort,
}

/// Terminal-independent UI state. Every plugin operation ends in App::invoke.
pub struct Ui {
    app: App,
    plugin_ids: Vec<String>,
    plugin: usize,
    groups: Vec<Group>,
    group: usize,
    items: Vec<Item>,
    location: Option<String>,
    view_title: String,
    parent: Option<CommandInvocation>,
    command_defaults: Vec<CommandInvocation>,
    history: BTreeMap<(String, String, String), ListState>,
    selected: usize,
    commands: Vec<Command>,
    focus: Focus,
    mode: Mode,
    filter: String,
    sort: Sort,
    pending: Option<char>,
    message: String,
    page_size: usize,
}

impl Ui {
    pub fn new(app: App) -> Self {
        let plugin_ids = app.plugins().into_iter().map(|plugin| plugin.id).collect();
        let commands = app.commands().into_iter().cloned().collect();
        let mut ui = Self {
            app,
            plugin_ids,
            plugin: 0,
            groups: Vec::new(),
            group: 0,
            items: Vec::new(),
            location: None,
            view_title: String::new(),
            parent: None,
            command_defaults: Vec::new(),
            history: BTreeMap::new(),
            selected: 0,
            commands,
            focus: Focus::Items,
            mode: Mode::Normal,
            filter: String::new(),
            sort: Sort::Plugin,
            pending: None,
            message: String::new(),
            page_size: 18,
        };
        ui.load_groups();
        ui
    }

    pub fn resize(&mut self, height: usize) {
        self.page_size = height.saturating_sub(6).max(1);
    }

    fn plugin_id(&self) -> Option<&str> {
        self.plugin_ids.get(self.plugin).map(String::as_str)
    }
    fn group_id(&self) -> Option<&str> {
        self.groups.get(self.group).map(|group| group.id.as_str())
    }

    fn visible_items(&self) -> Vec<&Item> {
        let query = self.filter.to_lowercase();
        let mut items: Vec<_> = self
            .items
            .iter()
            .filter(|item| item.title.to_lowercase().contains(&query))
            .collect();
        match self.sort {
            Sort::Plugin => {}
            Sort::Ascending => items.sort_by(|a, b| a.title.cmp(&b.title)),
            Sort::Descending => items.sort_by(|a, b| b.title.cmp(&a.title)),
        }
        items
    }

    fn selected_item(&self) -> Option<String> {
        self.visible_items()
            .get(self.selected)
            .map(|item| item.id.clone())
    }

    fn load_groups(&mut self) {
        self.group = 0;
        self.groups.clear();
        self.items.clear();
        self.location = None;
        self.view_title.clear();
        self.parent = None;
        self.command_defaults.clear();
        self.selected = 0;
        self.message.clear();
        if let Some(id) = self.plugin_id() {
            match self.app.groups(id) {
                Ok(groups) => self.groups = groups,
                Err(error) => self.message = error,
            }
        }
        self.refresh();
    }

    fn refresh(&mut self) {
        let previous = self.selected_item();
        if let (Some(plugin), Some(group)) = (self.plugin_id(), self.group_id()) {
            match self.app.view(plugin, group, self.location.as_deref()) {
                Ok(view) => {
                    self.apply_view(view, previous);
                    self.message.clear();
                }
                Err(error) => {
                    self.items.clear();
                    self.selected = 0;
                    self.message = error;
                }
            }
        }
    }

    fn apply_view(&mut self, view: GroupView, selected: Option<String>) {
        self.items = view.items;
        self.location = Some(view.location);
        self.view_title = view.title;
        self.parent = view.parent;
        self.command_defaults = view.command_defaults;
        self.selected = selected
            .and_then(|id| self.visible_items().iter().position(|item| item.id == id))
            .unwrap_or(0);
    }

    fn remember_location(&mut self) {
        if let (Some(plugin), Some(group), Some(location)) =
            (self.plugin_id(), self.group_id(), &self.location)
        {
            let key = (plugin.to_owned(), group.to_owned(), location.clone());
            let state = ListState {
                selected: self.selected_item(),
                filter: self.filter.clone(),
                sort: self.sort,
            };
            self.history.insert(key, state);
        }
    }

    fn open_location(&mut self, owner: &str, navigation: Navigation) -> Result<(), String> {
        let plugin = self
            .plugin_ids
            .iter()
            .position(|id| id == owner)
            .ok_or("Plugin not installed")?;
        let groups = self.app.groups(owner)?;
        let group = groups
            .iter()
            .position(|group| group.id == navigation.group)
            .ok_or("Unknown navigation group")?;
        // Load before changing state: a failed transition leaves the current list intact.
        let view = self
            .app
            .view(owner, &navigation.group, Some(&navigation.location))?;
        self.remember_location();
        let key = (owner.to_owned(), navigation.group, view.location.clone());
        let saved = self.history.get(&key);
        self.filter = saved.map_or(String::new(), |state| state.filter.clone());
        self.sort = saved.map_or(Sort::Plugin, |state| state.sort);
        let selected = navigation
            .selected
            .or_else(|| saved.and_then(|state| state.selected.clone()));
        if let Some(selected) = &selected {
            if view.items.iter().any(|item| {
                &item.id == selected
                    && !item
                        .title
                        .to_lowercase()
                        .contains(&self.filter.to_lowercase())
            }) {
                self.filter.clear();
            }
        }
        self.plugin = plugin;
        self.groups = groups;
        self.group = group;
        self.apply_view(view, selected);
        self.focus = Focus::Items;
        Ok(())
    }

    fn palette_commands(&self, query: &str) -> Vec<&Command> {
        let query = query.to_lowercase();
        self.commands
            .iter()
            .filter(|command| {
                command.id.to_lowercase().contains(&query)
                    || command.title.to_lowercase().contains(&query)
            })
            .collect()
    }

    fn invocation(&self, id: String, explicit_item: Option<String>) -> CommandInvocation {
        if explicit_item.is_none() {
            if let Some(default) = self
                .command_defaults
                .iter()
                .find(|invocation| invocation.id == id)
            {
                return default.clone();
            }
        }
        let item = explicit_item.or_else(|| {
            self.plugin_id()
                .filter(|plugin| id.starts_with(&format!("{plugin}.")))
                .filter(|_| {
                    self.commands
                        .iter()
                        .any(|command| command.id == id && command.requires_item)
                })
                .and_then(|_| self.selected_item())
        });
        CommandInvocation { id, item }
    }

    fn execute(&mut self, invocation: CommandInvocation) {
        let owner = self.app.command_owner(&invocation.id).map(str::to_owned);
        match self.app.invoke(invocation) {
            Ok(CommandOutcome::Output(block)) => {
                self.message.clear();
                self.mode = Mode::Output { block, offset: 0 };
            }
            Ok(CommandOutcome::Navigate(navigation)) => {
                self.mode = Mode::Normal;
                match self.open_location(owner.as_deref().unwrap_or(""), navigation) {
                    Ok(()) => self.message.clear(),
                    Err(error) => self.message = error,
                }
            }
            Err(error) => {
                self.message = error;
                self.mode = Mode::Normal;
            }
        }
    }

    fn show_actions(&mut self) {
        if let (Some(plugin), Some(group), Some(item)) =
            (self.plugin_id(), self.group_id(), self.selected_item())
        {
            match self
                .app
                .actions_at(plugin, group, self.location.as_deref(), &item)
            {
                Ok(entries) => {
                    self.mode = Mode::Actions {
                        entries,
                        selected: 0,
                    }
                }
                Err(error) => self.message = error,
            }
        } else {
            self.message = "Select an item first".into();
        }
    }

    fn activate_item(&mut self) {
        if let (Some(plugin), Some(group), Some(item)) =
            (self.plugin_id(), self.group_id(), self.selected_item())
        {
            match self
                .app
                .actions_at(plugin, group, self.location.as_deref(), &item)
            {
                Ok(entries) => {
                    if let Some(action) = entries.iter().find(|action| action.is_default) {
                        self.execute(action.invocation.clone());
                    } else {
                        self.mode = Mode::Actions {
                            entries,
                            selected: 0,
                        };
                    }
                }
                Err(error) => self.message = error,
            }
        } else {
            self.message = "Select an item first".into();
        }
    }

    fn go_parent(&mut self) {
        if let Some(invocation) = self.parent.clone() {
            self.execute(invocation);
        }
    }

    fn navigate(&mut self, forward: bool, count: usize) {
        match self.focus {
            Focus::Plugins => {
                let next = move_index(self.plugin, self.plugin_ids.len(), forward, count);
                if next != self.plugin {
                    self.remember_location();
                    self.plugin = next;
                    self.filter.clear();
                    self.load_groups();
                }
            }
            Focus::Groups => {
                let next = move_index(self.group, self.groups.len(), forward, count);
                if next != self.group {
                    self.remember_location();
                    self.group = next;
                    self.filter.clear();
                    self.selected = 0;
                    self.location = None;
                    self.items.clear();
                    self.view_title.clear();
                    self.parent = None;
                    self.command_defaults.clear();
                    self.refresh();
                }
            }
            Focus::Items => {
                self.selected =
                    move_index(self.selected, self.visible_items().len(), forward, count)
            }
        }
    }

    /// Returns false only for an explicit exit; errors remain visible in the session.
    pub fn handle(&mut self, key: Key) -> bool {
        if matches!(key, Key::Char('\x03' | '\x04')) {
            return false;
        }
        let mode = std::mem::replace(&mut self.mode, Mode::Normal);
        match mode {
            Mode::Normal => {
                if let Some(prefix) = self.pending.take() {
                    if let Key::Char(suffix) = key {
                        let sequence = format!("{prefix}{suffix}");
                        match BINDINGS.iter().find(|(keys, _)| *keys == sequence) {
                            Some((_, id)) => self.execute(self.invocation((*id).into(), None)),
                            None => {
                                self.message = format!("Unknown key sequence: {prefix} {suffix}")
                            }
                        }
                    }
                    return true;
                }
                match key {
                    Key::Char('q') => return false,
                    Key::Char(':') => self.mode = Mode::Command(String::new()),
                    Key::Char(' ') => {
                        self.mode = Mode::Palette {
                            query: String::new(),
                            selected: 0,
                        }
                    }
                    Key::Char('/') => {
                        self.mode = Mode::Search {
                            previous: self.filter.clone(),
                        }
                    }
                    Key::Char('?') => self.mode = Mode::Help { offset: 0 },
                    Key::Char('a') => self.show_actions(),
                    Key::Char('f') => self.pending = Some('f'),
                    Key::Char('r') => {
                        self.load_groups_preserving_selection();
                    }
                    Key::Char('s') => {
                        self.sort = match self.sort {
                            Sort::Plugin => Sort::Ascending,
                            Sort::Ascending => Sort::Descending,
                            Sort::Descending => Sort::Plugin,
                        };
                        self.selected = 0;
                    }
                    Key::Tab => self.focus = self.focus.next(),
                    Key::BackTab => self.focus = self.focus.previous(),
                    Key::ArrowUp | Key::Char('k') => self.navigate(false, 1),
                    Key::ArrowDown | Key::Char('j') => self.navigate(true, 1),
                    Key::PageUp => self.navigate(false, self.page_size),
                    Key::PageDown => self.navigate(true, self.page_size),
                    Key::Home => self.navigate(false, usize::MAX),
                    Key::End => self.navigate(true, usize::MAX),
                    Key::Backspace if self.focus == Focus::Items => self.go_parent(),
                    Key::ArrowLeft | Key::Char('h') => {
                        if self.focus == Focus::Items {
                            if self.parent.is_some() {
                                self.go_parent();
                            } else {
                                self.focus = Focus::Groups;
                            }
                        } else {
                            self.navigate(false, 1);
                        }
                    }
                    Key::ArrowRight | Key::Char('l') => {
                        if self.focus == Focus::Items {
                            self.activate_item();
                        } else {
                            self.navigate(true, 1);
                        }
                    }
                    Key::Enter => match self.focus {
                        Focus::Plugins => self.focus = Focus::Groups,
                        Focus::Groups => self.focus = Focus::Items,
                        Focus::Items => self.activate_item(),
                    },
                    Key::Escape => {
                        self.filter.clear();
                        self.selected = 0;
                        self.message.clear();
                    }
                    _ => {}
                }
            }
            Mode::Actions {
                entries,
                mut selected,
            } => {
                match key {
                    Key::Escape => return true,
                    Key::Enter => {
                        if let Some(action) = entries.get(selected) {
                            self.execute(action.invocation.clone());
                        }
                        return true;
                    }
                    Key::ArrowUp | Key::Char('k') => {
                        selected = move_index(selected, entries.len(), false, 1)
                    }
                    Key::ArrowDown | Key::Char('j') => {
                        selected = move_index(selected, entries.len(), true, 1)
                    }
                    _ => {}
                }
                self.mode = Mode::Actions { entries, selected };
            }
            Mode::Palette {
                mut query,
                mut selected,
            } => {
                match key {
                    Key::Escape => return true,
                    Key::Enter => {
                        if let Some(command) = self.palette_commands(&query).get(selected) {
                            self.execute(self.invocation(command.id.clone(), None));
                        }
                        return true;
                    }
                    Key::ArrowUp => {
                        selected =
                            move_index(selected, self.palette_commands(&query).len(), false, 1)
                    }
                    Key::ArrowDown => {
                        selected =
                            move_index(selected, self.palette_commands(&query).len(), true, 1)
                    }
                    Key::Backspace => {
                        query.pop();
                        selected = 0;
                    }
                    Key::Char(character) if !character.is_control() => {
                        query.push(character);
                        selected = 0;
                    }
                    _ => {}
                }
                self.mode = Mode::Palette { query, selected };
            }
            Mode::Command(mut input) => {
                match key {
                    Key::Escape => return true,
                    Key::Enter => {
                        let input = input.trim();
                        if !input.is_empty() {
                            let (id, item) =
                                input.split_once(' ').map_or((input, None), |(id, item)| {
                                    (
                                        id,
                                        if item.trim().is_empty() {
                                            None
                                        } else {
                                            Some(item.trim().into())
                                        },
                                    )
                                });
                            self.execute(self.invocation(id.into(), item));
                        }
                        return true;
                    }
                    Key::Backspace => {
                        input.pop();
                    }
                    Key::Char(character) if !character.is_control() => input.push(character),
                    _ => {}
                }
                self.mode = Mode::Command(input);
            }
            Mode::Search { previous } => {
                match key {
                    Key::Escape => {
                        self.filter = previous;
                        self.selected = 0;
                        return true;
                    }
                    Key::Enter => return true,
                    Key::Backspace => {
                        self.filter.pop();
                        self.selected = 0;
                    }
                    Key::Char(character) if !character.is_control() => {
                        self.filter.push(character);
                        self.selected = 0;
                    }
                    _ => {}
                }
                self.mode = Mode::Search { previous };
            }
            Mode::Output { block, mut offset } => {
                let max_offset = block.content.lines().count().saturating_sub(self.page_size);
                match key {
                    Key::Escape | Key::Enter => return true,
                    Key::Char('q') => return false,
                    Key::ArrowUp | Key::Char('k') => offset = offset.saturating_sub(1),
                    Key::ArrowDown | Key::Char('j') => {
                        offset = offset.saturating_add(1).min(max_offset)
                    }
                    Key::PageUp => offset = offset.saturating_sub(self.page_size),
                    Key::PageDown => offset = offset.saturating_add(self.page_size).min(max_offset),
                    Key::Home => offset = 0,
                    Key::End => offset = max_offset,
                    _ => {}
                }
                self.mode = Mode::Output { block, offset };
            }
            Mode::Help { mut offset } => {
                let max_offset = HELP.len().saturating_sub(self.page_size);
                match key {
                    Key::Char('q') => return false,
                    Key::Escape | Key::Enter | Key::Char('?') => return true,
                    Key::ArrowUp | Key::Char('k') => offset = offset.saturating_sub(1),
                    Key::ArrowDown | Key::Char('j') => {
                        offset = offset.saturating_add(1).min(max_offset)
                    }
                    Key::PageUp => offset = offset.saturating_sub(self.page_size),
                    Key::PageDown => offset = offset.saturating_add(self.page_size).min(max_offset),
                    Key::Home => offset = 0,
                    Key::End => offset = max_offset,
                    _ => {}
                }
                self.mode = Mode::Help { offset };
            }
        }
        true
    }

    fn load_groups_preserving_selection(&mut self) {
        if self.groups.is_empty() {
            self.load_groups();
        } else {
            self.refresh();
        }
    }

    /// Produces a bounded frame with terminal controls only from the renderer.
    pub fn render(&self, width: usize, height: usize) -> String {
        if width == 0 || height == 0 {
            return String::new();
        }
        let mut rows: Vec<(String, bool)> = vec![(String::new(), false); height];
        if height < 8 || width < 24 {
            rows[0].0 = "Terminal too small (minimum 24x8). q: quit".into();
        } else {
            rows[0].0 = format!(
                "Terminal Workspace | {}",
                self.app.workspace().root().display()
            );
            rows[1].0 = format!(
                "{} Plugins  {}",
                focus_marker(self.focus == Focus::Plugins),
                self.plugin_ids
                    .iter()
                    .enumerate()
                    .map(|(index, id)| if index == self.plugin {
                        format!("[{id}]")
                    } else {
                        id.clone()
                    })
                    .collect::<Vec<_>>()
                    .join("  ")
            );
            rows[2].0 = format!(
                "{} Groups   {}",
                focus_marker(self.focus == Focus::Groups),
                self.groups
                    .iter()
                    .enumerate()
                    .map(|(index, group)| if index == self.group {
                        format!("[{}]", group.title)
                    } else {
                        group.title.clone()
                    })
                    .collect::<Vec<_>>()
                    .join("  ")
            );
            let body_height = height - 6;
            let (heading, content, selection) = match &self.mode {
                Mode::Normal | Mode::Search { .. } | Mode::Command(_) => {
                    let items = self.visible_items();
                    (
                        format!(
                            "{} Items ({}/{}) | {} | order: {}",
                            focus_marker(self.focus == Focus::Items),
                            items.len(),
                            self.items.len(),
                            self.view_title,
                            match self.sort {
                                Sort::Plugin => "plugin",
                                Sort::Ascending => "title ascending",
                                Sort::Descending => "title descending",
                            }
                        ),
                        items
                            .iter()
                            .map(|item| item.title.clone())
                            .collect::<Vec<_>>(),
                        Some(self.selected),
                    )
                }
                Mode::Actions { entries, selected } => (
                    format!("Actions | {}", self.selected_item().unwrap_or_default()),
                    entries
                        .iter()
                        .map(|action| format!("{} — {}", action.label, action.command_id))
                        .collect(),
                    Some(*selected),
                ),
                Mode::Palette { query, selected } => (
                    format!("Command palette > {query}"),
                    self.palette_commands(query)
                        .iter()
                        .map(|command| format!("{} — {}", command.id, command.title))
                        .collect(),
                    Some(*selected),
                ),
                Mode::Output { block, offset } => (
                    format!(
                        "Output | {} | {} | line {}",
                        block.source,
                        block.status,
                        offset + 1
                    ),
                    block
                        .content
                        .lines()
                        .skip(*offset)
                        .map(str::to_owned)
                        .collect(),
                    None,
                ),
                Mode::Help { offset } => (
                    "Keyboard help (prototype defaults)".into(),
                    HELP.iter()
                        .skip(*offset)
                        .map(|line| (*line).to_owned())
                        .collect(),
                    None,
                ),
            };
            rows[3].0 = heading;
            let start = selection.map_or(0, |selected| selected.saturating_sub(body_height - 1));
            if content.is_empty() {
                rows[4].0 = "  No entries".into();
            }
            for (row, (index, text)) in content
                .iter()
                .enumerate()
                .skip(start)
                .take(body_height)
                .enumerate()
            {
                let highlighted = selection == Some(index);
                rows[4 + row] = (format!("{} {text}", focus_marker(highlighted)), highlighted);
            }
            rows[height - 2].0 = match &self.mode {
                Mode::Command(input) => format!(":{input}_"),
                Mode::Search { .. } => format!("/{}_", self.filter),
                _ if self.pending.is_some() => {
                    "f … o: open  p: preview  s: path  u: parent  Esc: cancel".into()
                }
                _ if !self.message.is_empty() => format!("Error: {}", self.message),
                _ => format!(
                    "{:?} | {} | filter: {}",
                    self.focus,
                    self.selected_item().unwrap_or_default(),
                    self.filter
                ),
            };
            rows[height - 1].0 = match self.mode {
                Mode::Actions { .. } => "↑↓ / j k: select   Enter: run   Esc: back".into(),
                Mode::Palette { .. } => {
                    "Type to filter   ↑↓: select   Enter: run   Esc: back".into()
                }
                Mode::Command(_) | Mode::Search { .. } => {
                    "Type text   Enter: accept   Esc: cancel".into()
                }
                Mode::Output { .. } => "↑↓ / j k: scroll   PgUp/PgDn   Esc: back   q: quit".into(),
                Mode::Help { .. } => "↑↓ / j k: scroll   Esc: back   q: quit".into(),
                Mode::Normal => {
                    "Enter: open/actions  a: all  Backspace: up  Space: palette  ?: help  q: quit"
                        .into()
                }
            };
        }
        let mut screen = String::from("\x1b[H");
        for (index, (text, highlighted)) in rows.into_iter().enumerate() {
            screen.push_str("\x1b[2K");
            if highlighted {
                screen.push_str("\x1b[7m");
            }
            // Reserve the final column so writing never triggers terminal autowrap.
            screen.push_str(&truncate_str(
                &safe_text(&text),
                width.saturating_sub(1),
                "",
            ));
            if highlighted {
                screen.push_str("\x1b[0m");
            }
            if index + 1 < height {
                screen.push_str("\r\n");
            }
        }
        screen
    }
}

fn move_index(index: usize, len: usize, forward: bool, count: usize) -> usize {
    if forward {
        index.saturating_add(count).min(len.saturating_sub(1))
    } else {
        index.saturating_sub(count)
    }
}

fn focus_marker(active: bool) -> &'static str {
    if active {
        ">"
    } else {
        " "
    }
}

fn safe_text(text: &str) -> String {
    text.replace('\t', "    ")
        .chars()
        .map(|character| {
            if character.is_control() {
                '�'
            } else {
                character
            }
        })
        .collect()
}
