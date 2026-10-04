use crate::{
    Action, App, Block, Command, CommandInvocation, CommandOutcome, Group, GroupView, Item,
    KeyBinding, Navigation,
};
use console::{truncate_str, Key};
use std::collections::BTreeMap;
use std::path::PathBuf;

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
    "Plugin bindings are local; see current bindings below",
    "Other item commands: a actions / Space palette / : commands",
    ", e/d/s   enable / disable / suspend selected plugin",
    ", p   plugin states; , g/r   grant / revoke WorkspaceRead",
    "a on Plugins   activation and permission actions",
    ", w   open Workspace; , b   previous Workspace",
    "PgUp/PgDn, Home/End   scroll list or output",
    "Esc   back; q / Ctrl-C / Ctrl-D   quit",
];

#[derive(Clone)]
struct ListState {
    selected: Option<String>,
    filter: String,
    sort: Sort,
}

struct WorkspaceUi {
    plugin: Option<String>,
    group: Option<String>,
    location: Option<String>,
    focus: Focus,
    history: BTreeMap<(String, String, String), ListState>,
}

/// Terminal-independent UI state. Every plugin operation ends in App::invoke.
pub struct Ui {
    app: App,
    plugin_ids: Vec<String>,
    plugin: usize,
    groups: Vec<Group>,
    group: usize,
    items: Vec<Item>,
    item_icons: BTreeMap<String, char>,
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
    pending: String,
    workspaces: BTreeMap<PathBuf, WorkspaceUi>,
    bindings: Vec<KeyBinding>,
    binding_errors: Vec<String>,
    message: String,
    page_size: usize,
    list_page_size: usize,
    output_item: Option<String>,
}

impl Ui {
    pub fn new(app: App) -> Self {
        let states = app.plugins();
        let plugin = states
            .iter()
            .position(|state| {
                state.status == "active"
                    && state.runtime.availability == crate::runtime::Availability::Available
            })
            .or_else(|| states.iter().position(|state| state.status == "active"))
            .unwrap_or(0);
        let plugin_ids = states.into_iter().map(|plugin| plugin.id).collect();
        let commands = app.commands().into_iter().cloned().collect();
        let mut ui = Self {
            app,
            plugin_ids,
            plugin,
            groups: Vec::new(),
            group: 0,
            items: Vec::new(),
            item_icons: BTreeMap::new(),
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
            pending: String::new(),
            workspaces: BTreeMap::new(),
            bindings: Vec::new(),
            binding_errors: Vec::new(),
            message: String::new(),
            page_size: 18,
            list_page_size: 18,
            output_item: None,
        };
        ui.load_groups();
        ui.update_bindings();
        if let Some(error) = ui.app.configuration_error() {
            ui.message = error.into();
        }
        ui
    }

    pub fn notify(&mut self, message: String) {
        self.message = message;
    }

    pub fn resize(&mut self, height: usize) {
        self.page_size = height.saturating_sub(6).max(1);
        self.list_page_size = self.page_size;
    }

    pub fn resize_to(&mut self, width: usize, height: usize) {
        self.page_size = viewport_rows(width, height);
        self.list_page_size = self.page_size;
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

    fn item_label(&self, item: &Item) -> String {
        format!(
            "{} {}",
            self.item_icons.get(&item.id).unwrap_or(&'•'),
            item.title
        )
    }

    fn selected_item(&self) -> Option<String> {
        self.visible_items()
            .get(self.selected)
            .map(|item| item.id.clone())
    }

    fn action_context(&self) -> String {
        if self.focus == Focus::Plugins {
            self.plugin_id().unwrap_or("No plugins").into()
        } else {
            self.selected_item().unwrap_or_default()
        }
    }

    fn load_groups(&mut self) {
        self.group = 0;
        self.groups.clear();
        self.items.clear();
        self.item_icons.clear();
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
        self.update_bindings();
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
                    self.item_icons.clear();
                    self.selected = 0;
                    self.message = error;
                }
            }
        }
    }

    fn apply_view(&mut self, view: GroupView, selected: Option<String>) {
        self.item_icons = view
            .items
            .iter()
            .map(|item| {
                (
                    item.id.clone(),
                    self.app
                        .item_icon(self.plugin_id().unwrap_or_default(), item),
                )
            })
            .collect();
        self.items = view.items;
        self.location = Some(view.location);
        self.view_title = view.title;
        self.parent = view.parent;
        self.command_defaults = view.command_defaults;
        self.selected = selected
            .and_then(|id| self.visible_items().iter().position(|item| item.id == id))
            .unwrap_or(0);
        self.update_bindings();
    }

    fn update_bindings(&mut self) {
        self.bindings =
            self.app
                .keybindings(self.plugin_id(), self.group_id(), self.location.as_deref());
        self.binding_errors = self.app.binding_diagnostics();
        self.pending.clear();
    }

    fn keyboard_help(&self) -> Vec<String> {
        let mut help: Vec<String> = HELP.iter().map(|line| (*line).into()).collect();
        help.push("Current context bindings:".into());
        help.extend(self.bindings.iter().map(|binding| {
            format!(
                "{}   {}",
                binding
                    .keys
                    .chars()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join(" "),
                binding.command_id
            )
        }));
        help.extend(
            self.binding_errors
                .iter()
                .map(|error| format!("Binding error: {error}")),
        );
        help.push("Esc   back; q / Ctrl-C / Ctrl-D   quit".into());
        help
    }

    fn pending_hint(&self) -> String {
        let continuations = self
            .bindings
            .iter()
            .filter_map(|binding| {
                binding
                    .keys
                    .strip_prefix(&self.pending)
                    .map(|suffix| format!("{suffix}: {}", binding.command_id))
            })
            .collect::<Vec<_>>()
            .join("  ");
        format!("{} … {continuations}  Esc: cancel", self.pending)
    }

    fn workspace_state(&mut self) -> WorkspaceUi {
        self.remember_location();
        WorkspaceUi {
            plugin: self.plugin_id().map(str::to_owned),
            group: self.group_id().map(str::to_owned),
            location: self.location.clone(),
            focus: self.focus,
            history: self.history.clone(),
        }
    }

    fn restore_workspace(&mut self) {
        self.history.clear();
        self.filter.clear();
        self.sort = Sort::Plugin;
        self.output_item = None;
        self.pending.clear();
        let states = self.app.plugins();
        self.plugin = states
            .iter()
            .position(|state| {
                state.status == "active"
                    && state.runtime.availability == crate::runtime::Availability::Available
            })
            .or_else(|| states.iter().position(|state| state.status == "active"))
            .unwrap_or(0);
        self.focus = Focus::Items;
        let saved = self.workspaces.remove(self.app.workspace().root());
        if let Some(state) = &saved {
            self.plugin = state
                .plugin
                .as_ref()
                .and_then(|id| self.plugin_ids.iter().position(|p| p == id))
                .unwrap_or(self.plugin);
        }
        self.load_groups();
        if let Some(state) = saved {
            self.history = state.history;
            self.focus = state.focus;
            if let (Some(plugin), Some(group), Some(location)) =
                (state.plugin, state.group, state.location)
            {
                if self.app.groups(&plugin).is_ok() {
                    if let Err(error) = self.open_location_inner(
                        &plugin,
                        Navigation {
                            group,
                            location,
                            selected: None,
                        },
                        false,
                    ) {
                        // Stale locations fall back to the root loaded above.
                        self.message = error;
                    }
                    self.focus = state.focus;
                }
            }
        }
        self.update_bindings();
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
        self.open_location_inner(owner, navigation, true)
    }

    fn open_location_inner(
        &mut self,
        owner: &str,
        navigation: Navigation,
        remember: bool,
    ) -> Result<(), String> {
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
        if remember {
            self.remember_location();
        }
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
        if id.starts_with("core.") {
            if matches!(
                id.as_str(),
                "core.permission.grant" | "core.permission.revoke"
            ) {
                return CommandInvocation {
                    id,
                    item: None,
                    args: explicit_item
                        .map(|args| args.split_whitespace().map(str::to_owned).collect())
                        .unwrap_or_default(),
                };
            }
            let args = if matches!(
                id.as_str(),
                "core.plugins"
                    | "core.plugins.discover"
                    | "core.plugin.install"
                    | "core.workspace.open"
                    | "core.workspace.previous"
            ) {
                explicit_item.into_iter().collect()
            } else {
                explicit_item
                    .or_else(|| self.plugin_id().map(str::to_owned))
                    .into_iter()
                    .collect()
            };
            return CommandInvocation {
                id,
                item: None,
                args,
            };
        }
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
        CommandInvocation {
            id,
            item,
            args: Vec::new(),
        }
    }

    fn execute(&mut self, invocation: CommandInvocation) {
        if invocation.id == "core.workspace.open" && invocation.args.is_empty() {
            self.mode = Mode::Command("core.workspace.open ".into());
            return;
        }
        if invocation.id == "core.plugin.install" && invocation.args.is_empty() {
            self.mode = Mode::Command("core.plugin.install ".into());
            return;
        }
        if matches!(
            invocation.id.as_str(),
            "core.permission.grant" | "core.permission.revoke"
        ) && invocation.args.len() < 2
        {
            let plugin = invocation
                .args
                .first()
                .map(String::as_str)
                .or_else(|| self.plugin_id())
                .unwrap_or_default();
            self.mode = Mode::Command(format!("{} {} ", invocation.id, plugin));
            return;
        }

        let workspace_change = invocation.id.starts_with("core.workspace.");
        let previous = if workspace_change {
            Some((
                self.app.workspace().root().to_owned(),
                self.workspace_state(),
            ))
        } else {
            None
        };
        let changes_state = matches!(
            invocation.id.as_str(),
            "core.plugin.enable"
                | "core.plugin.disable"
                | "core.plugin.suspend"
                | "core.permission.grant-read"
                | "core.permission.revoke-read"
                | "core.permission.grant"
                | "core.permission.revoke"
                | "core.plugin.install"
                | "core.plugins.discover"
                | "core.plugin.trust"
                | "core.plugin.untrust"
                | "core.plugin.uninstall"
                | "core.plugin.restart"
        );
        let owner = self.app.command_owner(&invocation.id).map(str::to_owned);
        let output_item = invocation.item.clone();
        match self.app.invoke(invocation) {
            Ok(CommandOutcome::WorkspaceChanged) => {
                if let Some((root, state)) = previous {
                    self.workspaces.insert(root, state);
                }
                self.mode = Mode::Normal;
                self.restore_workspace();
            }
            Ok(CommandOutcome::Output(block)) => {
                if changes_state {
                    let selected_plugin = self.plugin_id().map(str::to_owned);
                    self.plugin_ids = self
                        .app
                        .plugins()
                        .into_iter()
                        .map(|plugin| plugin.id)
                        .collect();
                    self.commands = self.app.commands().into_iter().cloned().collect();
                    self.plugin = selected_plugin
                        .and_then(|id| self.plugin_ids.iter().position(|plugin| plugin == &id))
                        .unwrap_or(0);
                    self.history.clear();
                    self.filter.clear();
                    self.load_groups();
                } else {
                    self.message.clear();
                }
                self.output_item = output_item;
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
        if self.focus == Focus::Plugins {
            let entries = self
                .commands
                .iter()
                .filter(|command| command.id.starts_with("core.") && command.id != "core.plugins")
                .map(|command| Action {
                    label: command.title.clone(),
                    command_id: command.id.clone(),
                    invocation: self.invocation(command.id.clone(), None),
                    is_default: false,
                })
                .collect();
            self.mode = Mode::Actions {
                entries,
                selected: 0,
            };
            return;
        }
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
                    self.update_bindings();
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
                if !self.pending.is_empty()
                    || matches!(&key, Key::Char(c) if self.bindings.iter().any(|b| b.keys.starts_with(*c)))
                {
                    if key == Key::Escape {
                        self.pending.clear();
                        self.message.clear();
                        return true;
                    }
                    if let Key::Char(character) = key {
                        self.pending.push(character);
                        if let Some(binding) = self
                            .bindings
                            .iter()
                            .find(|binding| binding.keys == self.pending)
                        {
                            let id = binding.command_id.clone();
                            self.pending.clear();
                            self.execute(self.invocation(id, None));
                        } else if !self
                            .bindings
                            .iter()
                            .any(|binding| binding.keys.starts_with(&self.pending))
                        {
                            self.message = format!("Unknown key sequence: {}", self.pending);
                            self.pending.clear();
                        }
                    } else {
                        self.pending.clear();
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
                    Key::PageUp => self.navigate(false, self.list_page_size),
                    Key::PageDown => self.navigate(true, self.list_page_size),
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
                let max_offset = self.keyboard_help().len().saturating_sub(self.page_size);
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
        if width >= 90 && height >= 16 {
            return self.render_split(width, height);
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
                            .map(|item| self.item_label(item))
                            .collect::<Vec<_>>(),
                        Some(self.selected),
                    )
                }
                Mode::Actions { entries, selected } => (
                    format!("Actions | {}", self.action_context()),
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
                    self.keyboard_help().into_iter().skip(*offset).collect(),
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
                _ if !self.pending.is_empty() => self.pending_hint(),
                _ if !self.message.is_empty() => format!("Error: {}", self.message),
                _ if !self.binding_errors.is_empty() => {
                    format!("Bindings: {} (? for details)", self.binding_errors[0])
                }
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

    fn render_split(&self, width: usize, height: usize) -> String {
        let width = width - 1; // Keep the terminal's autowrap column unused.
        let rich = height >= 24;
        let left = (width * 44 / 100).max(32);
        let right = width - left - 1;
        let body_height = viewport_rows(width + 1, height);
        let list_height = body_height;
        let items = self.visible_items();
        let start = self.selected.saturating_sub(list_height - 1);
        let selected = items.get(self.selected);
        let mut rows = Vec::with_capacity(height);
        let summary = self
            .app
            .plugins()
            .into_iter()
            .find(|plugin| Some(plugin.id.as_str()) == self.plugin_id())
            .map_or_else(
                || "No plugins".into(),
                |plugin| format!("{} · {}", plugin.id, plugin.status),
            );
        rows.push(workspace_header(
            &self.app.workspace().root().display().to_string(),
            &summary,
            width,
        ));
        if rich {
            rows.push(styled_cell(&"─".repeat(width), width, ACCENT));
            rows.push(styled_cell(&panel_rule(width, '╭', '╮'), width, BORDER));
        }
        let plugins = tab_row(
            &format!("{} Plugins  ", focus_marker(self.focus == Focus::Plugins)),
            self.plugin_ids
                .iter()
                .enumerate()
                .map(|(index, id)| (id.as_str(), index == self.plugin)),
            if rich { width - 4 } else { width },
        );
        rows.push(if rich {
            framed_row(&plugins, width)
        } else {
            plugins
        });
        if rich {
            rows.push(styled_cell(&panel_rule(width, '╰', '╯'), width, BORDER));
            rows.push(styled_cell(&panel_rule(width, '╭', '╮'), width, BORDER));
        }

        let groups = tab_row(
            &format!("{} Groups   ", focus_marker(self.focus == Focus::Groups)),
            self.groups
                .iter()
                .enumerate()
                .map(|(index, group)| (group.title.as_str(), index == self.group)),
            if rich { width - 4 } else { width },
        );
        rows.push(if rich {
            framed_row(&groups, width)
        } else {
            groups
        });
        if rich {
            rows.push(styled_cell(&panel_rule(width, '╰', '╯'), width, BORDER));
        }

        rows.push(styled_cell(
            &format!(
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
            width,
            MUTED,
        ));
        rows.push(panel_pair(
            &panel_rule(left, '╭', '╮'),
            &panel_rule(right, '╭', '╮'),
            BORDER,
            BORDER,
        ));

        let (heading, content, selection) = match &self.mode {
            Mode::Actions { entries, selected } => (
                format!("Actions | {}", self.action_context()),
                entries
                    .iter()
                    .map(|action| format!("{} — {}", action.label, action.command_id))
                    .collect::<Vec<_>>(),
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
                self.keyboard_help().into_iter().skip(*offset).collect(),
                None,
            ),
            _ => (
                selected.map_or("Selection".into(), |item| item.title.clone()),
                selected.map_or_else(
                    || {
                        vec![
                            "No entries".into(),
                            "Space   command palette".into(),
                            ", e     enable selected plugin".into(),
                            "Tab, a  plugin actions".into(),
                        ]
                    },
                    |item| {
                        vec![
                            format!("Kind: {}", item.kind),
                            format!("Item: {}", item.id),
                            String::new(),
                            "Enter   default action".into(),
                            "a       contextual actions".into(),
                            "Space   command palette".into(),
                            String::new(),
                            "Command results appear here.".into(),
                        ]
                    },
                ),
                None,
            ),
        };
        rows.push(panel_pair(
            &panel_cell(
                &format!(
                    "{}  ·  {} items",
                    self.groups
                        .get(self.group)
                        .map_or("Items", |group| group.title.as_str()),
                    items.len()
                ),
                left,
                ACCENT,
            ),
            &panel_cell(
                if rich && matches!(self.mode, Mode::Output { .. }) {
                    self.output_item.as_deref().unwrap_or(&heading)
                } else {
                    &heading
                },
                right,
                TITLE,
            ),
            "",
            "",
        ));
        if rich {
            let metadata = match &self.mode {
                Mode::Output { .. } => heading.clone(),
                _ => selected.map_or_else(
                    || "No selection".into(),
                    |item| format!("{}  ·  {}", item.kind, self.plugin_id().unwrap_or_default()),
                ),
            };
            rows.push(panel_pair(
                &panel_cell(
                    &format!(" {} visible / {} total", items.len(), self.items.len()),
                    left,
                    MUTED,
                ),
                &panel_cell(&metadata, right, MUTED),
                "",
                "",
            ));
            rows.push(panel_pair(
                &panel_cell(&"─".repeat(left - 2), left, BORDER),
                &panel_cell(&"─".repeat(right - 2), right, BORDER),
                "",
                "",
            ));
        }
        let content_start = selection.map_or(0, |index| index.saturating_sub(body_height - 1));
        for row in 0..body_height {
            let index = start + row;
            let item_text = items
                .get(index)
                .map(|item| {
                    let label = self.item_label(item);
                    if rich {
                        format!(
                            " {:02} {} {}",
                            index + 1,
                            focus_marker(index == self.selected),
                            label
                        )
                    } else {
                        format!("{} {}", focus_marker(index == self.selected), label)
                    }
                })
                .unwrap_or_else(|| {
                    if row == 0 && items.is_empty() {
                        "  No entries".into()
                    } else {
                        String::new()
                    }
                });
            let detail_index = content_start + row;
            let detail = content.get(detail_index).map_or(String::new(), |line| {
                if selection.is_some() {
                    format!("{} {line}", focus_marker(selection == Some(detail_index)))
                } else if rich {
                    if let Mode::Output { offset, .. } = self.mode {
                        format!(" {:>3}  {line}", offset + row + 1)
                    } else {
                        format!("  {line}")
                    }
                } else {
                    line.clone()
                }
            });
            rows.push(panel_pair(
                &panel_cell(
                    &item_text,
                    left,
                    if index == self.selected && !items.is_empty() {
                        SELECTED
                    } else {
                        TEXT
                    },
                ),
                &panel_cell(
                    &detail,
                    right,
                    if selection == Some(detail_index) {
                        SELECTED
                    } else {
                        TEXT
                    },
                ),
                "",
                "",
            ));
        }
        rows.push(panel_pair(
            &panel_rule(left, '╰', '╯'),
            &panel_rule(right, '╰', '╯'),
            BORDER,
            BORDER,
        ));
        let status = match &self.mode {
            Mode::Command(input) => format!(":{input}_"),
            Mode::Search { .. } => format!("/{}_", self.filter),
            _ if !self.pending.is_empty() => self.pending_hint(),
            _ if !self.message.is_empty() => format!("Error: {}", self.message),
            _ if !self.binding_errors.is_empty() => {
                format!("Bindings: {} (? for details)", self.binding_errors[0])
            }
            _ => format!(
                "{:?} | {} | filter: {}",
                self.focus,
                self.selected_item().unwrap_or_default(),
                self.filter
            ),
        };
        rows.push(styled_cell(
            &status,
            width,
            if self.message.is_empty() {
                MUTED
            } else {
                ERROR
            },
        ));
        if rich {
            rows.push(styled_cell(&panel_rule(width, '╭', '╮'), width, BORDER));
        }
        let hints: &[(&str, &str)] = match self.mode {
            Mode::Normal => &[
                ("↑↓", "Select"),
                ("Tab", "Focus"),
                ("Enter", "Open"),
                ("a", "Actions"),
                ("/", "Search"),
                ("Space", "Commands"),
                ("?", "Help"),
                ("q", "Quit"),
            ],
            Mode::Output { .. } | Mode::Help { .. } => &[
                ("↑↓", "Scroll"),
                ("PgUp/PgDn", "Page"),
                ("Esc", "Back"),
                ("q", "Quit"),
            ],
            Mode::Actions { .. } => &[("↑↓", "Select action"), ("Enter", "Run"), ("Esc", "Back")],
            Mode::Palette { .. } => &[
                ("Type", "Filter"),
                ("↑↓", "Select"),
                ("Enter", "Run"),
                ("Esc", "Back"),
            ],
            _ => &[("Type", "Text"), ("Enter", "Accept"), ("Esc", "Cancel")],
        };
        let hints = hint_row(hints, if rich { width - 4 } else { width });
        rows.push(if rich {
            framed_row(&hints, width)
        } else {
            hints
        });
        if rich {
            rows.push(styled_cell(&panel_rule(width, '╰', '╯'), width, BORDER));
        }
        let mut screen = String::from("\x1b[H");
        for (index, row) in rows.iter().enumerate() {
            screen.push_str("\x1b[2K");
            screen.push_str(row);
            screen.push_str("\x1b[0m");
            if index + 1 < rows.len() {
                screen.push_str("\r\n");
            }
        }
        screen
    }
}

const TEXT: &str = "\x1b[38;2;211;221;239m\x1b[48;2;3;16;24m";
const MUTED: &str = "\x1b[38;2;135;165;200m\x1b[48;2;3;16;24m";
const ACCENT: &str = "\x1b[38;2;70;190;235m\x1b[48;2;3;16;24m";
const BORDER: &str = "\x1b[38;2;38;117;155m\x1b[48;2;3;16;24m";
const SELECTED: &str = "\x1b[38;2;230;211;255m\x1b[48;2;43;30;76m";
const ERROR: &str = "\x1b[38;2;255;160;135m\x1b[48;2;3;16;24m";
const TITLE: &str = "\x1b[1m\x1b[38;2;230;235;245m\x1b[48;2;3;16;24m";
const PURPLE: &str = "\x1b[1m\x1b[38;2;171;92;245m\x1b[48;2;3;16;24m";
const KEYCAP: &str = "\x1b[38;2;228;232;245m\x1b[48;2;20;44;65m";

fn viewport_rows(width: usize, height: usize) -> usize {
    if width >= 90 && height >= 24 {
        height - 18
    } else if width >= 90 && height >= 16 {
        height - 9
    } else {
        height.saturating_sub(6).max(1)
    }
}

fn workspace_header(path: &str, status: &str, width: usize) -> String {
    let status_width = console::measure_text_width(&safe_text(status)).min(width / 3);
    let path_width = width.saturating_sub(23 + status_width);
    format!(
        "{}{}{}{}{}",
        styled_cell("Terminal ", 9, ACCENT),
        styled_cell("Workspace", 9, PURPLE),
        styled_cell("  │  ", 5, BORDER),
        styled_cell(path, path_width, ACCENT),
        styled_cell(status, status_width, MUTED),
    )
}

fn framed_row(content: &str, width: usize) -> String {
    debug_assert_eq!(console::measure_text_width(content), width - 4);
    format!("{BORDER}│{TEXT} {content}{TEXT} {BORDER}│\x1b[0m")
}

fn hint_row(hints: &[(&str, &str)], width: usize) -> String {
    let mut row = String::new();
    let mut remaining = width;
    for (key, label) in hints {
        let key_width = console::measure_text_width(key) + 2;
        let label = format!(" {label}  ");
        let label_width = console::measure_text_width(&label);
        if key_width + label_width > remaining {
            break;
        }
        row.push_str(&styled_cell(&format!(" {key} "), key_width, KEYCAP));
        row.push_str(&styled_cell(&label, label_width, TEXT));
        remaining -= key_width + label_width;
    }
    row.push_str(&styled_cell("", remaining, TEXT));
    row
}

fn styled_cell(text: &str, width: usize, style: &str) -> String {
    let text = safe_text(text);
    let text = truncate_str(&text, width, "");
    let padding = width.saturating_sub(console::measure_text_width(&text));
    format!("{style}{text}{}\x1b[0m", " ".repeat(padding))
}

fn tab_row<'a>(prefix: &str, tabs: impl Iterator<Item = (&'a str, bool)>, width: usize) -> String {
    let mut row = styled_cell(
        prefix,
        width.min(console::measure_text_width(prefix)),
        MUTED,
    );
    let mut remaining = width.saturating_sub(console::measure_text_width(prefix));
    for (title, active) in tabs {
        let title = safe_text(title);
        let label = if active { format!("[{title}]") } else { title };
        let size = (console::measure_text_width(&label) + 3).min(remaining);
        row.push_str(&styled_cell(
            &label,
            size,
            if active { SELECTED } else { TEXT },
        ));
        remaining -= size;
        if remaining == 0 {
            break;
        }
    }
    row.push_str(&styled_cell("", remaining, TEXT));
    row
}

fn panel_cell(text: &str, width: usize, style: &str) -> String {
    format!(
        "{BORDER}│{TEXT} {}{TEXT} {BORDER}│\x1b[0m",
        styled_cell(text, width - 4, style),
    )
}

fn panel_rule(width: usize, first: char, last: char) -> String {
    format!("{first}{}{last}", "─".repeat(width - 2))
}

fn panel_pair(left: &str, right: &str, left_style: &str, right_style: &str) -> String {
    format!("{left_style}{left}{TEXT} {right_style}{right}\x1b[0m")
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
