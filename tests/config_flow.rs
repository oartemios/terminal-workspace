use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use terminal_workspace::{files::FilesPlugin, App, CommandInvocation, Permission, CONFIG_FILE};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Project(PathBuf);
impl Project {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "tw-config-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn open(&self) -> App {
        let mut app = App::open(self.0.clone()).unwrap();
        app.install(Box::new(FilesPlugin), true).unwrap();
        app.grant_default("files", Permission::WorkspaceRead)
            .unwrap();
        app
    }
    fn command(app: &mut App, id: &str, args: &[&str]) -> Result<(), String> {
        app.invoke(CommandInvocation {
            id: id.into(),
            item: None,
            args: args.iter().map(|value| (*value).into()).collect(),
        })
        .map(|_| ())
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn activation_and_permission_decisions_survive_restart_independently_per_project() {
    let first = Project::new();
    let second = Project::new();
    let mut app = first.open();
    let before = std::fs::read(first.0.join(CONFIG_FILE)).unwrap();
    Project::command(&mut app, "core.plugin.suspend", &["files"]).unwrap();
    assert_eq!(app.plugins()[0].status, "session disabled");
    assert!(app.items("files", "entries").is_err());
    assert_eq!(std::fs::read(first.0.join(CONFIG_FILE)).unwrap(), before);
    let mut restarted = first.open();
    assert_eq!(restarted.plugins()[0].status, "active");
    Project::command(&mut restarted, "core.plugin.disable", &["files"]).unwrap();
    let mut restarted = first.open();
    assert_eq!(restarted.plugins()[0].status, "workspace disabled");
    assert_eq!(second.open().plugins()[0].status, "active");
    Project::command(&mut restarted, "core.plugin.enable", &["files"]).unwrap();
    assert!(restarted.items("files", "entries").is_ok());
    Project::command(&mut restarted, "core.permission.revoke-read", &["files"]).unwrap();
    let mut restarted = first.open();
    assert!(restarted
        .items("files", "entries")
        .unwrap_err()
        .contains("WorkspaceRead"));
    Project::command(&mut restarted, "core.permission.grant-read", &["files"]).unwrap();
    assert!(first.open().items("files", "entries").is_ok());
}

#[test]
fn unknown_entries_settings_and_overrides_are_preserved_by_core_changes() {
    let project = Project::new();
    let original = serde_json::json!({"version":1,"plugins":{"files":{"enabled":true,"permissions":["WorkspaceRead","FuturePermission"],"settings":{"show":"all"},"future":42},"missing":{"enabled":true,"settings":{"hello":"world"}}},"overrides":{"future-binding":"x"},"future-root":true});
    std::fs::write(project.0.join(CONFIG_FILE), original.to_string()).unwrap();
    let mut app = project.open();
    assert_eq!(
        app.workspace().plugin_settings("files").unwrap()["show"],
        "all"
    );
    app.disable("files").unwrap();
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(project.0.join(CONFIG_FILE)).unwrap()).unwrap();
    let mut expected = original;
    expected["plugins"]["files"]["enabled"] = false.into();
    assert_eq!(saved, expected);
    assert!(
        Project::command(&mut app, "core.plugin.enable", &["missing"])
            .unwrap_err()
            .contains("not installed")
    );
}

#[test]
fn malformed_or_future_configuration_stays_untouched_and_core_remains_available() {
    let project = Project::new();
    for text in [
        "{broken",
        "{\"version\":2,\"plugins\":{}}",
        "{\"version\":1,\"plugins\":{\"files\":{\"enabled\":\"yes\"}}}",
    ] {
        std::fs::write(project.0.join(CONFIG_FILE), text).unwrap();
        let mut app = project.open();
        assert!(app.configuration_error().is_some());
        assert_eq!(app.plugins()[0].status, "workspace disabled");
        Project::command(&mut app, "core.plugins", &[]).unwrap();
        assert!(app.enable("files").is_err());
        assert_eq!(
            std::fs::read_to_string(project.0.join(CONFIG_FILE)).unwrap(),
            text
        );
    }
}

#[test]
fn failed_save_does_not_change_activation_or_permissions() {
    let project = Project::new();
    let mut app = project.open();
    let path = project.0.join(CONFIG_FILE);
    let changed = "{\"version\":1,\"plugins\":{}}";
    std::fs::write(&path, changed).unwrap();
    assert!(app
        .disable("files")
        .unwrap_err()
        .contains("changed on disk"));
    assert_eq!(app.plugins()[0].status, "active");
    assert!(app.revoke("files", Permission::WorkspaceRead).is_err());
    assert!(app.items("files", "entries").is_ok());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), changed);
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(app.disable("files").is_err());
    assert_eq!(app.plugins()[0].status, "active");
}

#[test]
fn configuration_symlink_is_rejected_without_touching_its_target() {
    let project = Project::new();
    let outside = Project::new();
    let target = outside.0.join("settings.json");
    std::fs::write(&target, "{\"version\":1,\"plugins\":{}}").unwrap();
    std::os::unix::fs::symlink(&target, project.0.join(CONFIG_FILE)).unwrap();
    let mut app = project.open();
    assert!(app.configuration_error().unwrap().contains("symlinks"));
    assert!(app.enable("files").is_err());
    assert_eq!(
        std::fs::read_to_string(target).unwrap(),
        "{\"version\":1,\"plugins\":{}}"
    );
}

#[test]
fn core_registry_works_without_plugins_or_selected_items() {
    let project = Project::new();
    let mut app = App::open(project.0.clone()).unwrap();
    Project::command(&mut app, "core.plugins", &[]).unwrap();
    assert!(Project::command(&mut app, "core.plugin.enable", &[])
        .unwrap_err()
        .contains("requires one plugin id"));
    assert!(
        Project::command(&mut app, "core.plugin.suspend", &["unknown"])
            .unwrap_err()
            .contains("not installed")
    );
}

#[test]
fn unavailable_project_directory_does_not_apply_an_unsaved_change() {
    let project = Project::new();
    let mut app = App::open(project.0.clone()).unwrap();
    app.install(Box::new(FilesPlugin), true).unwrap();
    let moved = project.0.with_extension("moved");
    std::fs::rename(&project.0, &moved).unwrap();
    assert!(app.disable("files").is_err());
    assert_eq!(app.plugins()[0].status, "active");
    assert!(app.grant("files", Permission::WorkspaceRead).is_err());
    std::fs::rename(&moved, &project.0).unwrap();
    assert!(app
        .items("files", "entries")
        .unwrap_err()
        .contains("WorkspaceRead"));
    app.grant("files", Permission::WorkspaceRead).unwrap();
    assert!(app.items("files", "entries").is_ok());
}
