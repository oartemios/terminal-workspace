use std::path::PathBuf;
use terminal_workspace::{files::FilesPlugin, App, CommandInvocation, CommandOutcome, Permission};

fn workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn action_and_colon_command_share_the_same_execution_path() {
    let mut app = App::new(workspace()).unwrap();
    app.install(Box::new(FilesPlugin), true).unwrap();
    app.grant("files", Permission::WorkspaceRead).unwrap();
    assert_eq!(app.groups("files").unwrap()[0].id, "entries");
    let item = app
        .items("files", "entries")
        .unwrap()
        .into_iter()
        .find(|item| item.id == "Cargo.toml")
        .unwrap();
    let action = app
        .actions("files", "entries", &item.id)
        .unwrap()
        .into_iter()
        .find(|action| action.command_id == "files.preview")
        .unwrap();
    let from_action = app.invoke(action.invocation).unwrap();
    let from_command = app
        .invoke(CommandInvocation {
            id: "files.preview".into(),
            item: Some("Cargo.toml".into()),
            args: Vec::new(),
        })
        .unwrap();
    assert_eq!(from_action, from_command);
    let CommandOutcome::Output(block) = from_action else {
        panic!("Expected output");
    };
    assert!(block.content.contains("terminal-workspace"));
}

#[test]
fn session_disable_does_not_change_workspace_activation() {
    let mut app = App::new(workspace()).unwrap();
    app.install(Box::new(FilesPlugin), true).unwrap();
    app.grant("files", Permission::WorkspaceRead).unwrap();
    app.disable_for_session("files").unwrap();
    assert_eq!(app.plugins()[0].status, "session disabled");
    assert!(app.groups("files").is_err());
    app.enable("files").unwrap();
    assert_eq!(app.plugins()[0].status, "active");
    app.disable("files").unwrap();
    assert_eq!(app.plugins()[0].status, "workspace disabled");
}

#[test]
fn path_resolution_stays_inside_workspace() {
    let app = App::new(workspace()).unwrap();
    assert!(app.workspace().read_path("Cargo.toml").is_ok());
    assert!(app.workspace().read_path("/etc/hosts").is_err());
}

#[test]
fn requested_permission_must_be_granted_before_loading_items() {
    let mut app = App::new(workspace()).unwrap();
    app.install(Box::new(FilesPlugin), true).unwrap();
    assert!(app
        .items("files", "entries")
        .unwrap_err()
        .contains("WorkspaceRead"));
    app.grant("files", Permission::WorkspaceRead).unwrap();
    assert!(app.items("files", "entries").is_ok());
}
