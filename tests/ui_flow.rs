use console::{measure_text_width, strip_ansi_codes, Key};
use std::fmt::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use terminal_workspace::{
    files::FilesPlugin, ui::Ui, Action, App, Block, Command, CommandInvocation, CommandOutcome,
    Group, Item, Permission, Plugin, Workspace,
};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "tw-ui-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn ui(&self) -> Ui {
        let mut app = App::new(self.0.clone()).unwrap();
        app.install(Box::new(FilesPlugin), true).unwrap();
        app.grant("files", Permission::WorkspaceRead).unwrap();
        Ui::new(app)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn type_text(ui: &mut Ui, text: &str) {
    for character in text.chars() {
        assert!(ui.handle(Key::Char(character)));
    }
}

fn frame(ui: &Ui) -> String {
    strip_ansi_codes(&ui.render(100, 24)).into_owned()
}

fn select(ui: &mut Ui, name: &str) {
    ui.handle(Key::Char('/'));
    type_text(ui, name);
    ui.handle(Key::Enter);
}

#[test]
fn all_four_command_entry_points_render_the_same_plugin_result() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.0.join("note with spaces.md"),
        "shared result from Files",
    )
    .unwrap();
    for route in 0..4 {
        let mut ui = fixture.ui();
        // Traverse plugin and group focus entirely through keyboard input.
        ui.handle(Key::Tab);
        ui.handle(Key::Enter);
        ui.handle(Key::Enter);
        select(&mut ui, "note with spaces.md");
        match route {
            0 => {
                ui.handle(Key::Enter);
                ui.handle(Key::Char('j'));
                ui.handle(Key::Enter);
            }
            1 => {
                type_text(&mut ui, "fp");
            }
            2 => {
                ui.handle(Key::Char(':'));
                type_text(&mut ui, "files.preview note with spaces.md");
                ui.handle(Key::Enter);
            }
            _ => {
                ui.handle(Key::Char(' '));
                type_text(&mut ui, "preview");
                ui.handle(Key::Enter);
            }
        }
        let output = frame(&ui);
        assert!(
            output.contains("Output | Files | ok"),
            "route {route}: {output}"
        );
        assert!(output.contains("shared result from Files"));
        ui.handle(Key::Escape);
        assert!(frame(&ui).contains("note with spaces.md"));
    }
}

#[test]
fn stale_item_error_does_not_exit_and_refresh_recovers() {
    let fixture = Fixture::new();
    let path = fixture.0.join("removed.txt");
    std::fs::write(&path, "before deletion").unwrap();
    let mut ui = fixture.ui();
    std::fs::remove_file(&path).unwrap();
    assert!(ui.handle(Key::Enter));
    assert!(frame(&ui).contains("Error: Item not found"));
    ui.handle(Key::Char('r'));
    assert!(frame(&ui).contains("No entries"));
    assert!(!frame(&ui).contains("Error:"));
    assert!(!ui.handle(Key::Char('q')));
}

#[test]
fn unicode_filter_cancel_and_empty_group_are_usable() {
    let fixture = Fixture::new();
    let mut ui = fixture.ui();
    assert!(frame(&ui).contains("[Entries]"));
    ui.handle(Key::Enter);
    assert!(frame(&ui).contains("Select an item first"));
    std::fs::write(fixture.0.join("заметка.md"), "текст").unwrap();
    ui.handle(Key::Char('r'));
    ui.handle(Key::Char('/'));
    type_text(&mut ui, "нет совпадения");
    assert!(frame(&ui).contains("No entries"));
    ui.handle(Key::Escape);
    assert!(frame(&ui).contains("заметка.md"));
    select(&mut ui, "заметка");
    type_text(&mut ui, "fp");
    assert!(frame(&ui).contains("текст"));
}

#[test]
fn frame_is_bounded_and_file_controls_cannot_change_the_terminal() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.0.join("wide.txt"),
        "界".repeat(100) + "\n\x1b[2J\ttext\x07",
    )
    .unwrap();
    let mut ui = fixture.ui();
    type_text(&mut ui, "fp");
    let screen = ui.render(30, 8);
    // Terminal control bytes in the content are replaced before rendering.
    assert!(!screen.contains("\x1b[2J"));
    assert!(!screen.contains('\x07'));
    let plain = strip_ansi_codes(&screen);
    assert_eq!(plain.lines().count(), 8);
    for line in plain.lines() {
        assert!(measure_text_width(line) < 30);
    }
    assert!(strip_ansi_codes(&ui.render(10, 3)).contains("Terminal"));
}

#[test]
fn long_list_and_output_scroll_keep_the_selection_visible() {
    let fixture = Fixture::new();
    let mut content = String::new();
    for line in 0..50 {
        writeln!(&mut content, "line {line:02}").unwrap();
    }
    for index in 0..50 {
        std::fs::write(fixture.0.join(format!("file-{index:02}.txt")), &content).unwrap();
    }
    let mut ui = fixture.ui();
    ui.resize(10);
    ui.handle(Key::End);
    assert!(strip_ansi_codes(&ui.render(60, 10)).contains("> file-49.txt"));
    type_text(&mut ui, "fp");
    ui.handle(Key::End);
    let output = strip_ansi_codes(&ui.render(60, 10)).into_owned();
    assert!(output.contains("line 49"));
    ui.handle(Key::Escape);
    assert!(strip_ansi_codes(&ui.render(60, 10)).contains("> file-49.txt"));
}

struct StatusPlugin;

impl Plugin for StatusPlugin {
    fn id(&self) -> &'static str {
        "status"
    }
    fn name(&self) -> &'static str {
        "Status"
    }
    fn groups(&self) -> Vec<Group> {
        vec![Group {
            id: "info".into(),
            title: "Info".into(),
        }]
    }
    fn items(&self, _: &Workspace, _: &str) -> Result<Vec<Item>, String> {
        Ok(Vec::new())
    }
    fn actions(&self, _: &Item) -> Vec<Action> {
        Vec::new()
    }
    fn commands(&self) -> Vec<Command> {
        vec![Command {
            id: "status.summary".into(),
            title: "Workspace summary".into(),
            requires_item: false,
        }]
    }
    fn execute(&self, _: &Workspace, _: &CommandInvocation) -> Result<CommandOutcome, String> {
        Ok(CommandOutcome::Output(Block {
            source: "Status".into(),
            status: "ok".into(),
            content: "command without an item".into(),
        }))
    }
}

#[test]
fn switching_plugins_and_palette_without_an_item_use_the_public_contract() {
    let fixture = Fixture::new();
    let mut app = App::new(fixture.0.clone()).unwrap();
    app.install(Box::new(FilesPlugin), true).unwrap();
    app.grant("files", Permission::WorkspaceRead).unwrap();
    app.install(Box::new(StatusPlugin), true).unwrap();
    let mut ui = Ui::new(app);
    ui.handle(Key::Tab);
    ui.handle(Key::ArrowRight);
    ui.handle(Key::Enter);
    assert!(frame(&ui).contains("[status]"));
    assert!(frame(&ui).contains("[Info]"));
    ui.handle(Key::Enter);
    ui.handle(Key::Char(' '));
    type_text(&mut ui, "summary");
    ui.handle(Key::Enter);
    assert!(frame(&ui).contains("command without an item"));
}

#[test]
fn inactive_plugin_and_small_terminal_help_do_not_block_navigation() {
    let fixture = Fixture::new();
    let mut app = App::new(fixture.0.clone()).unwrap();
    app.install(Box::new(FilesPlugin), false).unwrap();
    let mut ui = Ui::new(app);
    assert!(frame(&ui).contains("Plugin inactive: files"));
    ui.resize(8);
    ui.handle(Key::Char('?'));
    ui.handle(Key::End);
    assert!(strip_ansi_codes(&ui.render(80, 8)).contains("Ctrl-C / Ctrl-D"));
    ui.handle(Key::Escape);
    assert!(!ui.handle(Key::Char('\x03')));
}

#[test]
fn directory_commands_share_navigation_and_restore_parent_selection() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.0.join("notes")).unwrap();
    std::fs::write(fixture.0.join("notes/заметка.md"), "nested preview").unwrap();
    for route in 0..5 {
        let mut ui = fixture.ui();
        select(&mut ui, "notes");
        match route {
            0 => {
                ui.handle(Key::Enter);
            }
            1 => {
                ui.handle(Key::Char('a'));
                ui.handle(Key::Enter);
            }
            2 => type_text(&mut ui, "fo"),
            3 => {
                ui.handle(Key::Char(':'));
                type_text(&mut ui, "files.open notes");
                ui.handle(Key::Enter);
            }
            _ => {
                ui.handle(Key::Char(' '));
                type_text(&mut ui, "Open directory");
                ui.handle(Key::Enter);
            }
        }
        assert!(
            frame(&ui).contains("| /notes/ |"),
            "route {route}: {}",
            frame(&ui)
        );
        assert!(frame(&ui).contains("> заметка.md"));
        type_text(&mut ui, "fp");
        assert!(frame(&ui).contains("nested preview"));
        ui.handle(Key::Escape);
        match route {
            0 => {
                ui.handle(Key::Backspace);
            }
            1 => {
                ui.handle(Key::Char('h'));
            }
            2 => type_text(&mut ui, "fu"),
            3 => {
                ui.handle(Key::Char(':'));
                type_text(&mut ui, "files.parent");
                ui.handle(Key::Enter);
            }
            _ => {
                ui.handle(Key::Char(' '));
                type_text(&mut ui, "Parent directory");
                ui.handle(Key::Enter);
            }
        }
        assert!(frame(&ui).contains("| / |"));
        assert!(frame(&ui).contains("> notes/"));
        assert!(frame(&ui).contains("filter: notes"));
        // A parent operation at root stays inside the workspace.
        type_text(&mut ui, "fu");
        assert!(frame(&ui).contains("| / |"));
        assert!(!frame(&ui).contains("Error:"));
    }
}

#[test]
fn revisiting_nested_directories_restores_filter_and_selection() {
    let fixture = Fixture::new();
    std::fs::create_dir_all(fixture.0.join("notes/chapter")).unwrap();
    std::fs::write(fixture.0.join("notes/a.md"), "a").unwrap();
    std::fs::write(fixture.0.join("notes/z.md"), "z").unwrap();
    let mut ui = fixture.ui();
    ui.handle(Key::Enter);
    select(&mut ui, "z.md");
    ui.handle(Key::Backspace);
    ui.handle(Key::Enter);
    assert!(frame(&ui).contains("filter: z.md"));
    assert!(frame(&ui).contains("> z.md"));
    ui.handle(Key::Escape);
    select(&mut ui, "chapter");
    ui.handle(Key::Enter);
    assert!(frame(&ui).contains("No entries"));
    assert!(frame(&ui).contains("| /notes/chapter/ |"));
    ui.handle(Key::Backspace);
    assert!(frame(&ui).contains("> chapter/"));
    ui.handle(Key::Backspace);
    assert!(frame(&ui).contains("> notes/"));
}

#[test]
fn missing_directory_refresh_allows_return_to_parent() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.0.join("deleted")).unwrap();
    let mut ui = fixture.ui();
    ui.handle(Key::Enter);
    std::fs::remove_dir(fixture.0.join("deleted")).unwrap();
    ui.handle(Key::Char('r'));
    assert!(frame(&ui).contains("Error:"));
    ui.handle(Key::Backspace);
    assert!(frame(&ui).contains("| / |"));
    assert!(!frame(&ui).contains("Error:"));
}

#[test]
fn internal_symlink_returns_logically_and_external_symlink_is_rejected() {
    let fixture = Fixture::new();
    let outside = Fixture::new();
    std::fs::create_dir_all(fixture.0.join("nested/target")).unwrap();
    std::fs::write(fixture.0.join("nested/target/inside.txt"), "inside").unwrap();
    std::os::unix::fs::symlink(fixture.0.join("nested/target"), fixture.0.join("alias")).unwrap();
    std::os::unix::fs::symlink(outside.0.as_path(), fixture.0.join("external")).unwrap();
    let mut ui = fixture.ui();
    select(&mut ui, "alias");
    ui.handle(Key::Enter);
    assert!(frame(&ui).contains("| /alias/ |"));
    ui.handle(Key::Backspace);
    assert!(frame(&ui).contains("> alias/"));
    for command in ["files.open external", "files.open ../", "files.parent ../"] {
        ui.handle(Key::Char(':'));
        type_text(&mut ui, command);
        ui.handle(Key::Enter);
        assert!(frame(&ui).contains("Error: Path is outside the workspace"));
        assert!(frame(&ui).contains("| / |"));
    }
}
