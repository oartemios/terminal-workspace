use console::{strip_ansi_codes, Key};
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use terminal_workspace::{
    files::FilesPlugin, ui::Ui, App, CommandInvocation, CommandOutcome, Permission, CONFIG_FILE,
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Project(PathBuf);
impl Project {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "tw-switch-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn app(&self) -> App {
        let mut app = App::open(self.0.clone()).unwrap();
        app.install(Box::new(FilesPlugin), true).unwrap();
        app.grant_default("files", Permission::WorkspaceRead)
            .unwrap();
        app
    }
    fn config(&self, value: serde_json::Value) {
        std::fs::write(self.0.join(CONFIG_FILE), value.to_string()).unwrap();
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn invoke(app: &mut App, id: &str, args: Vec<String>) -> Result<CommandOutcome, String> {
    app.invoke(CommandInvocation {
        id: id.into(),
        item: None,
        args,
    })
}
fn open(app: &mut App, project: &Project) {
    assert_eq!(
        invoke(
            app,
            "core.workspace.open",
            vec![project.0.to_string_lossy().into_owned()]
        )
        .unwrap(),
        CommandOutcome::WorkspaceChanged
    );
}
fn text(ui: &mut Ui, value: &str) {
    for key in value.chars() {
        assert!(ui.handle(Key::Char(key)));
    }
}
fn frame(ui: &Ui) -> String {
    strip_ansi_codes(&ui.render(160, 30)).into_owned()
}
fn command(ui: &mut Ui, value: &str) {
    text(ui, &format!(":{value}"));
    ui.handle(Key::Enter);
}

#[test]
fn switching_preserves_installation_and_isolates_activation_permissions_and_settings() {
    let first = Project::new();
    let second = Project::new();
    std::fs::write(first.0.join("first.txt"), "first").unwrap();
    std::fs::write(second.0.join("second.txt"), "second").unwrap();
    second.config(json!({"version":1,"plugins":{"files":{"enabled":true,"permissions":[],"settings":{"marker":"second"}}},"overrides":{}}));
    let mut app = first.app();
    app.disable_for_session("files").unwrap();
    let unchanged = std::fs::read(first.0.join(CONFIG_FILE)).unwrap();
    open(&mut app, &second);
    assert_eq!(app.plugins()[0].status, "active");
    assert_eq!(
        app.workspace().plugin_settings("files").unwrap()["marker"],
        "second"
    );
    assert!(app
        .items("files", "entries")
        .unwrap_err()
        .contains("WorkspaceRead"));
    app.grant("files", Permission::WorkspaceRead).unwrap();
    assert!(app
        .items("files", "entries")
        .unwrap()
        .iter()
        .any(|item| item.id == "second.txt"));
    app.disable("files").unwrap();
    open(&mut app, &first);
    assert_eq!(app.plugins()[0].status, "session disabled");
    assert_eq!(std::fs::read(first.0.join(CONFIG_FILE)).unwrap(), unchanged);
    app.enable("files").unwrap();
    assert!(app
        .items("files", "entries")
        .unwrap()
        .iter()
        .any(|item| item.id == "first.txt"));
    assert!(app.workspace().plugin_settings("files").is_none());
    invoke(&mut app, "core.workspace.previous", vec![]).unwrap();
    assert_eq!(app.plugins()[0].status, "workspace disabled");
    assert_eq!(second.app().plugins()[0].status, "workspace disabled");
}

#[test]
fn bad_path_configuration_and_default_save_failure_leave_current_workspace_usable() {
    let first = Project::new();
    let second = Project::new();
    let mut app = first.app();
    for path in [first.0.join("missing"), first.0.join(CONFIG_FILE)] {
        assert!(invoke(
            &mut app,
            "core.workspace.open",
            vec![path.to_string_lossy().into_owned()]
        )
        .is_err());
        assert!(app.items("files", "entries").is_ok());
    }
    std::fs::write(second.0.join(CONFIG_FILE), "{broken").unwrap();
    assert!(invoke(
        &mut app,
        "core.workspace.open",
        vec![second.0.to_string_lossy().into_owned()]
    )
    .is_err());
    assert_eq!(app.workspace().root(), first.0.canonicalize().unwrap());
    assert!(invoke(&mut app, "core.workspace.previous", vec![])
        .unwrap_err()
        .contains("No previous"));
    std::fs::remove_file(second.0.join(CONFIG_FILE)).unwrap();
    // Valid compact input fits the limit, but saving defaults would exceed it.
    second.config(json!({"version":1,"plugins":{},"padding":"x".repeat(1_048_470)}));
    assert!(app
        .switch_workspace(second.0.clone())
        .unwrap_err()
        .contains("exceeds 1 MiB"));
    assert!(app.items("files", "entries").is_ok());
}

#[test]
fn keyboard_routes_switch_and_restore_project_context_without_cached_data() {
    for route in 0..4 {
        let first = Project::new();
        let second = Project::new();
        std::fs::create_dir(first.0.join("notes")).unwrap();
        std::fs::write(first.0.join("notes/z.txt"), "first output").unwrap();
        std::fs::write(first.0.join("notes/a.txt"), "another item").unwrap();
        std::fs::write(second.0.join("second.txt"), "second output").unwrap();
        let mut ui = Ui::new(first.app());
        command(&mut ui, "files.open notes");
        text(&mut ui, "ss");
        text(&mut ui, "/z.txt");
        ui.handle(Key::Enter);
        text(&mut ui, "p");
        assert!(frame(&ui).contains("first output"));
        ui.handle(Key::Escape);
        match route {
            0 => text(&mut ui, ",w"),
            1 => text(&mut ui, ":core.workspace.open "),
            2 => {
                text(&mut ui, " core.workspace.open");
                ui.handle(Key::Enter);
            }
            _ => {
                ui.handle(Key::Tab);
                text(&mut ui, "a");
                for _ in 0..6 {
                    ui.handle(Key::ArrowDown);
                }
                ui.handle(Key::Enter);
            }
        }
        assert!(
            frame(&ui).contains(":core.workspace.open "),
            "route {route}: {}",
            frame(&ui)
        );
        text(&mut ui, &second.0.to_string_lossy());
        ui.handle(Key::Enter);
        let rendered = frame(&ui);
        assert!(rendered.contains("second.txt"), "route {route}: {rendered}");
        assert!(!rendered.contains("first output"));
        assert!(!rendered.contains("z.txt"));
        text(&mut ui, "/second.txt");
        ui.handle(Key::Enter);
        text(&mut ui, "p");
        assert!(frame(&ui).contains("second output"));
        ui.handle(Key::Escape);
        std::fs::write(first.0.join("notes/z.txt"), "updated first output").unwrap();
        text(&mut ui, ",b");
        assert!(frame(&ui).contains("/notes/"));
        assert!(frame(&ui).contains("filter: z.txt"));
        assert!(strip_ansi_codes(&ui.render(80, 24)).contains("order: title descending"));
        text(&mut ui, "p");
        assert!(frame(&ui).contains("updated first output"));
        ui.handle(Key::Escape);
        command(&mut ui, "core.workspace.open missing");
        assert!(frame(&ui).contains("Error:"));
        assert!(frame(&ui).contains("/notes/"));
    }
}

#[test]
fn stale_saved_location_falls_back_and_switching_with_no_plugins_works() {
    let first = Project::new();
    let second = Project::new();
    std::fs::create_dir(first.0.join("removed")).unwrap();
    let mut ui = Ui::new(first.app());
    command(&mut ui, "files.open removed");
    command(
        &mut ui,
        &format!("core.workspace.open {}", second.0.display()),
    );
    std::fs::remove_dir(first.0.join("removed")).unwrap();
    text(&mut ui, ",b");
    assert!(frame(&ui).contains("/ |"));
    assert!(frame(&ui).contains("Error:"));
    let mut ui = Ui::new(App::new(first.0.clone()).unwrap());
    text(&mut ui, ",w");
    text(&mut ui, &second.0.to_string_lossy());
    ui.handle(Key::Enter);
    assert!(frame(&ui).contains(&second.0.to_string_lossy().to_string()));
    text(&mut ui, ",b");
    assert!(frame(&ui).contains(&first.0.to_string_lossy().to_string()));
}

#[test]
fn overrides_follow_workspace_scope_and_long_sequences_are_discoverable_and_cancellable() {
    let first = Project::new();
    let second = Project::new();
    std::fs::write(first.0.join("note.txt"), "override preview").unwrap();
    std::fs::create_dir(first.0.join("notes")).unwrap();
    std::fs::write(first.0.join("notes/child.txt"), "nested override preview").unwrap();
    std::fs::write(second.0.join("other.txt"), "second preview").unwrap();
    first.config(json!({"version":1,"plugins":{},"overrides":{"keybindings":[
        {"keys":"p","plugin":"files","command":null},
        {"keys":"fp","plugin":"files","command":"files.preview"},
        {"keys":"xyz界","plugin":"files","command":"files.preview"},
        {"keys":"v","plugin":"files","command":"files.path"},
        {"keys":"v","plugin":"files","group":"entries","location":"notes","command":"files.preview"},
        {"keys":"y","plugin":"files","group":"entries","command":"files.path"},
        {"keys":"y","plugin":"files","group":"entries","location":"notes","command":null}
    ]}}));
    let app = first.app();
    assert!(app
        .keybindings(Some("files"), Some("entries"), Some("."))
        .iter()
        .any(|b| b.keys == "y"));
    assert!(!app
        .keybindings(Some("files"), Some("entries"), Some("notes"))
        .iter()
        .any(|b| b.keys == "y"));
    let mut ui = Ui::new(app);
    text(&mut ui, "/note.txt");
    ui.handle(Key::Enter);
    text(&mut ui, "p");
    assert!(!frame(&ui).contains("override preview"));
    // Removed defaults remain valid explicit user overrides.
    text(&mut ui, "fp");
    assert!(frame(&ui).contains("override preview"));
    ui.handle(Key::Escape);
    text(&mut ui, "xy");
    for (width, height) in [(160, 30), (80, 24)] {
        let rendered = strip_ansi_codes(&ui.render(width, height)).into_owned();
        assert!(rendered.contains("xy … z界: files.preview"));
    }
    ui.handle(Key::Escape);
    text(&mut ui, "z界");
    assert!(!frame(&ui).contains("override preview"));
    text(&mut ui, "xyz界");
    assert!(frame(&ui).contains("override preview"));
    ui.handle(Key::Escape);
    text(&mut ui, "/xyz界");
    ui.handle(Key::Enter);
    assert!(frame(&ui).contains("No entries"));
    ui.handle(Key::Escape);
    command(&mut ui, "files.open notes");
    text(&mut ui, "v");
    assert!(frame(&ui).contains("nested override preview"));
    ui.handle(Key::Escape);
    command(
        &mut ui,
        &format!("core.workspace.open {}", second.0.display()),
    );
    text(&mut ui, "x");
    assert!(!frame(&ui).contains("second preview"));
    text(&mut ui, "/other.txt");
    ui.handle(Key::Enter);
    text(&mut ui, "p");
    assert!(frame(&ui).contains("second preview"));
    ui.handle(Key::Escape);
    text(&mut ui, ",bxyz界");
    assert!(frame(&ui).contains("nested override preview"));
}

#[test]
fn conflicting_or_malformed_bindings_are_reported_without_blocking_core_or_valid_bindings() {
    let project = Project::new();
    std::fs::write(project.0.join("note.txt"), "valid preview").unwrap();
    project.config(json!({"version":1,"plugins":{},"overrides":{"keybindings":[
        {"keys":"f","plugin":"files","command":"files.preview"},
        {"keys":"fp","plugin":"files","command":"files.path"},
        {"keys":"x","plugin":"files","command":"files.path"},
        {"keys":"x","plugin":"files","command":"files.preview"},
        {"keys":"j","plugin":"files","command":"files.preview"},
        {"keys":"z","plugin":"files","command":"missing.command"},
        {"keys":34,"command":null}
    ]}}));
    let app = project.app();
    let errors = app.binding_diagnostics().join("\n");
    assert!(errors.contains("Ambiguous binding prefix: f / fp"));
    assert!(errors.contains("Duplicate binding override: x"));
    assert!(errors.contains("reserved"));
    assert!(errors.contains("Unknown binding command"));
    assert!(!app
        .keybindings(Some("files"), Some("entries"), Some("."))
        .iter()
        .any(|b| b.keys.starts_with('f') || b.keys == "x"));
    let mut ui = Ui::new(app);
    assert!(frame(&ui).contains("Bindings:"));
    text(&mut ui, "/note.txt");
    ui.handle(Key::Enter);
    text(&mut ui, "p");
    assert!(frame(&ui).contains("valid preview"));
    ui.handle(Key::Escape);
    text(&mut ui, ",s");
    assert!(frame(&ui).contains("session disabled"));
    ui.handle(Key::Escape);
    text(&mut ui, "p");
    assert!(!frame(&ui).contains("valid preview"));
    text(&mut ui, ",e");
    ui.handle(Key::Escape);
    text(&mut ui, "/note.txt");
    ui.handle(Key::Enter);
    text(&mut ui, "p");
    assert!(frame(&ui).contains("valid preview"));
}
