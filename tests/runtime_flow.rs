use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use terminal_workspace::{
    files::FilesPlugin,
    runtime::{write_package, Availability, Connection, Descriptor, Manifest, PackageStore},
    App, Command, CommandInvocation, CommandOutcome, Group, Permission, CONFIG_FILE,
    PLUGIN_API_VERSION,
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    base: PathBuf,
    first: PathBuf,
    second: PathBuf,
    store: PackageStore,
}
impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "tw-runtime-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&base).unwrap();
        let base = base.canonicalize().unwrap();
        let first = base.join("first");
        let second = base.join("second");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        std::fs::write(first.join("note.txt"), "first file").unwrap();
        std::fs::write(second.join("note.txt"), "second file").unwrap();
        let store = PackageStore::new(base.join("global-plugins")).unwrap();
        store
            .bootstrap(
                &FilesPlugin,
                Path::new(env!("CARGO_BIN_EXE_tw")),
                vec!["--serve-files".into()],
            )
            .unwrap();
        Self {
            base,
            first,
            second,
            store,
        }
    }
    fn app(&self) -> App {
        let mut app = App::open(self.first.clone()).unwrap();
        assert!(app.load_packages(self.store.clone(), &["files"]).is_empty());
        app.grant_default("files", Permission::WorkspaceRead)
            .unwrap();
        app
    }
    fn probe(&self, permissions: Vec<Permission>) -> PathBuf {
        let source = self.base.join("probe-package");
        std::fs::create_dir(&source).unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/runtime_plugin.py"),
            source.join("plugin"),
        )
        .unwrap();
        let manifest = Manifest {
            package_version: 1,
            protocol_version: 1,
            api_version: PLUGIN_API_VERSION.into(),
            executable: "plugin".into(),
            args: Vec::new(),
            environment: Vec::new(),
            credentials: Vec::new(),
            plugin: Descriptor {
                id: "probe".into(),
                name: "Probe".into(),
                groups: vec![Group {
                    id: "entries".into(),
                    title: "Entries".into(),
                }],
                commands: [
                    "info",
                    "error",
                    "crash",
                    "hang",
                    "malformed",
                    "oversized",
                    "wrong_id",
                    "child",
                ]
                .into_iter()
                .map(|name| Command {
                    id: format!("probe.{name}"),
                    title: name.into(),
                    requires_item: false,
                })
                .collect(),
                keybindings: Vec::new(),
                permissions,
            },
        };
        std::fs::write(
            source.join("plugin.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        source
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}
fn invoke(app: &mut App, id: &str, args: Vec<String>) -> Result<CommandOutcome, String> {
    app.invoke(CommandInvocation {
        id: id.into(),
        item: None,
        args,
    })
}
fn info(app: &mut App) -> Value {
    let CommandOutcome::Output(block) = invoke(app, "probe.info", vec![]).unwrap() else {
        panic!("Expected output");
    };
    serde_json::from_str(&block.content).unwrap()
}
fn files(app: &mut App) -> String {
    let CommandOutcome::Output(block) = app
        .invoke(CommandInvocation {
            id: "files.preview".into(),
            item: Some("note.txt".into()),
            args: vec![],
        })
        .unwrap()
    else {
        panic!("Expected output");
    };
    block.content
}
fn not_running(pid: u32) -> bool {
    // SAFETY: signal 0 only probes the given test child pid.
    if unsafe { libc::kill(pid as i32, 0) } != 0 {
        return true;
    }
    let status = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let status = String::from_utf8_lossy(&status.stdout);
    status.trim().is_empty() || status.trim().starts_with('Z')
}

#[test]
fn package_install_discovery_trust_permissions_and_workspace_switch_are_independent() {
    let fixture = Fixture::new();
    let source = fixture.probe(vec![Permission::WorkspaceRead]);
    std::fs::write(source.join(".trusted"), b"trusted-local-executable-v1\n").unwrap();
    let mut app = fixture.app();
    invoke(
        &mut app,
        "core.plugin.install",
        vec![source.to_string_lossy().into_owned()],
    )
    .unwrap();
    let state = app.plugins().into_iter().find(|p| p.id == "probe").unwrap();
    assert!(state.installed);
    assert!(!state.workspace_enabled);
    assert_eq!(state.runtime.availability, Availability::Untrusted);
    assert_eq!(state.runtime.connection, Connection::Disconnected);
    app.enable("probe").unwrap();
    assert!(invoke(&mut app, "probe.info", vec![])
        .unwrap_err()
        .contains("WorkspaceRead"));
    app.grant("probe", Permission::WorkspaceRead).unwrap();
    assert!(invoke(&mut app, "probe.info", vec![])
        .unwrap_err()
        .contains("untrusted"));
    app.trust_plugin("probe").unwrap();
    let first = info(&mut app);
    let pid = first["pid"].as_u64().unwrap() as u32;
    assert_eq!(first["root"], fixture.first.to_string_lossy().as_ref());
    app.switch_workspace(fixture.second.clone()).unwrap();
    assert!(not_running(pid));
    assert_eq!(
        app.plugins()
            .iter()
            .find(|p| p.id == "probe")
            .unwrap()
            .status,
        "workspace disabled"
    );
    assert_eq!(files(&mut app), "second file");
    app.enable("probe").unwrap();
    assert!(invoke(&mut app, "probe.info", vec![]).is_err());
    app.grant("probe", Permission::WorkspaceRead).unwrap();
    let second = info(&mut app);
    assert_eq!(second["root"], fixture.second.to_string_lossy().as_ref());
    assert_ne!(first["pid"], second["pid"]);
    app.switch_workspace(fixture.first.clone()).unwrap();
    assert_eq!(files(&mut app), "first file");
    assert_eq!(
        info(&mut app)["root"],
        fixture.first.to_string_lossy().as_ref()
    );
    let saved = std::fs::read(fixture.first.join(CONFIG_FILE)).unwrap();
    app.uninstall("probe").unwrap();
    assert_eq!(
        std::fs::read(fixture.first.join(CONFIG_FILE)).unwrap(),
        saved
    );
    assert!(invoke(&mut app, "probe.info", vec![])
        .unwrap_err()
        .contains("Unknown command"));
    assert!(!fixture.store.directory("probe").unwrap().exists());
    app.install_package(source).unwrap();
    assert_eq!(
        app.plugins()
            .iter()
            .find(|p| p.id == "probe")
            .unwrap()
            .status,
        "active"
    );
    assert!(!fixture.store.is_trusted("probe"));
}

#[test]
fn suspend_disable_revoke_and_drop_terminate_worker_and_children() {
    let fixture = Fixture::new();
    let source = fixture.probe(vec![Permission::Process]);
    let mut app = fixture.app();
    app.install_package(source).unwrap();
    app.trust_plugin("probe").unwrap();
    app.enable("probe").unwrap();
    app.grant("probe", Permission::Process).unwrap();
    let CommandOutcome::Output(block) = invoke(&mut app, "probe.child", vec![]).unwrap() else {
        panic!();
    };
    let child: Value = serde_json::from_str(&block.content).unwrap();
    let pid = child["pid"].as_u64().unwrap() as u32;
    let child_pid = child["child"].as_u64().unwrap() as u32;
    let before = std::fs::read(fixture.first.join(CONFIG_FILE)).unwrap();
    app.disable_for_session("probe").unwrap();
    assert!(not_running(pid));
    for _ in 0..20 {
        if not_running(child_pid) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(not_running(child_pid));
    assert_eq!(
        std::fs::read(fixture.first.join(CONFIG_FILE)).unwrap(),
        before
    );
    app.enable("probe").unwrap();
    let pid = info(&mut app)["pid"].as_u64().unwrap() as u32;
    app.revoke("probe", Permission::Process).unwrap();
    assert!(not_running(pid));
    assert!(invoke(&mut app, "probe.info", vec![])
        .unwrap_err()
        .contains("Process"));
    app.grant("probe", Permission::Process).unwrap();
    let pid = info(&mut app)["pid"].as_u64().unwrap() as u32;
    app.disable("probe").unwrap();
    assert!(not_running(pid));
    app.enable("probe").unwrap();
    let pid = info(&mut app)["pid"].as_u64().unwrap() as u32;
    drop(app);
    assert!(not_running(pid));
}

#[test]
fn runtime_failures_are_bounded_and_localized_and_domain_errors_do_not_disconnect() {
    let fixture = Fixture::new();
    let mut app = fixture.app();
    app.install_package(fixture.probe(vec![])).unwrap();
    app.trust_plugin("probe").unwrap();
    app.enable("probe").unwrap();
    let original = info(&mut app)["pid"].clone();
    assert!(invoke(&mut app, "probe.error", vec![])
        .unwrap_err()
        .contains("domain error"));
    assert_eq!(info(&mut app)["pid"], original);
    assert!(app
        .actions("probe", "entries", "probe.item")
        .unwrap_err()
        .contains("own registered command"));
    assert_eq!(
        app.plugins()
            .iter()
            .find(|p| p.id == "files")
            .unwrap()
            .status,
        "active"
    );
    for command in ["crash", "hang", "malformed", "oversized", "wrong_id"] {
        let pid = info(&mut app)["pid"].as_u64().unwrap() as u32;
        let start = std::time::Instant::now();
        assert!(invoke(&mut app, &format!("probe.{command}"), vec![]).is_err());
        assert!(start.elapsed() < std::time::Duration::from_secs(3));
        assert!(not_running(pid));
        assert_eq!(files(&mut app), "first file");
        let state = app.plugins().into_iter().find(|p| p.id == "probe").unwrap();
        assert_eq!(state.runtime.connection, Connection::Failed);
        invoke(&mut app, "core.plugin.restart", vec!["probe".into()]).unwrap();
    }
}

#[test]
fn every_declared_permission_is_gated_and_only_explicit_environment_names_are_forwarded() {
    let fixture = Fixture::new();
    let permissions = vec![
        Permission::WorkspaceRead,
        Permission::WorkspaceWrite,
        Permission::Process,
        Permission::Network,
        Permission::Credentials,
        Permission::Environment,
    ];
    let source = fixture.probe(permissions.clone());
    let mut manifest = Manifest::read(&source).unwrap();
    manifest.environment = vec!["TW_RUNTIME_TEST_ENV".into()];
    manifest.credentials = vec!["TW_RUNTIME_TEST_CREDENTIAL".into()];
    std::fs::write(
        source.join("plugin.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    std::env::set_var("TW_RUNTIME_TEST_ENV", "explicit environment");
    std::env::set_var("TW_RUNTIME_TEST_CREDENTIAL", "fake credential for test");
    std::env::set_var("TW_UNDECLARED_SECRET", "not forwarded");
    let mut app = fixture.app();
    app.install_package(source).unwrap();
    app.trust_plugin("probe").unwrap();
    app.enable("probe").unwrap();
    for permission in permissions {
        assert!(invoke(&mut app, "probe.info", vec![]).is_err());
        invoke(
            &mut app,
            "core.permission.grant",
            vec!["probe".into(), permission.name()],
        )
        .unwrap();
    }
    let result = info(&mut app);
    assert_eq!(
        result["environment"]["TW_RUNTIME_TEST_ENV"],
        "explicit environment"
    );
    assert_eq!(
        result["environment"]["TW_RUNTIME_TEST_CREDENTIAL"],
        "fake credential for test"
    );
    assert!(result["environment"]["TW_UNDECLARED_SECRET"].is_null());
    assert!(result["environment"]["HOME"].is_null());
    for permission in [
        Permission::WorkspaceRead,
        Permission::WorkspaceWrite,
        Permission::Process,
        Permission::Network,
        Permission::Credentials,
        Permission::Environment,
    ] {
        app.revoke("probe", permission).unwrap();
        assert!(invoke(&mut app, "probe.info", vec![]).is_err());
        app.grant("probe", permission).unwrap();
    }
    std::env::remove_var("TW_RUNTIME_TEST_ENV");
    std::env::remove_var("TW_RUNTIME_TEST_CREDENTIAL");
    std::env::remove_var("TW_UNDECLARED_SECRET");
}

#[test]
fn compatibility_invalid_packages_and_missing_executables_do_not_break_files() {
    let fixture = Fixture::new();
    let source = fixture.probe(vec![]);
    let mut manifest = Manifest::read(&source).unwrap();
    manifest.api_version = "99".into();
    std::fs::write(
        source.join("plugin.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    assert!(fixture
        .store
        .install(&source)
        .unwrap_err()
        .contains("Incompatible"));
    let installed = fixture.store.directory("probe").unwrap();
    std::fs::create_dir(&installed).unwrap();
    std::fs::write(
        installed.join("plugin.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let mut app = fixture.app();
    app.enable("probe").unwrap();
    assert!(invoke(&mut app, "probe.info", vec![])
        .unwrap_err()
        .contains("Incompatible"));
    assert_eq!(
        app.plugins()
            .iter()
            .find(|p| p.id == "probe")
            .unwrap()
            .runtime
            .availability,
        Availability::Incompatible
    );
    assert_eq!(files(&mut app), "first file");
    manifest.api_version = PLUGIN_API_VERSION.into();
    std::fs::write(
        installed.join("plugin.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    drop(app);
    let mut app = fixture.app();
    assert_eq!(
        app.plugins()
            .iter()
            .find(|p| p.id == "probe")
            .unwrap()
            .runtime
            .availability,
        Availability::Missing
    );
    assert_eq!(files(&mut app), "first file");
    let invalid = fixture.store.root().join("broken");
    std::fs::create_dir(&invalid).unwrap();
    std::fs::write(invalid.join("plugin.json"), "{broken").unwrap();
    assert!(!app.load_packages(fixture.store.clone(), &[]).is_empty());
    assert_eq!(files(&mut app), "first file");
}

#[test]
fn a_worker_with_different_metadata_is_rejected_before_domain_calls() {
    let fixture = Fixture::new();
    let source = fixture.probe(vec![]);
    let mut manifest = Manifest::read(&source).unwrap();
    manifest.args = vec!["mismatch".into()];
    std::fs::write(
        source.join("plugin.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let mut app = fixture.app();
    app.install_package(source).unwrap();
    app.trust_plugin("probe").unwrap();
    app.enable("probe").unwrap();
    assert!(invoke(&mut app, "probe.info", vec![])
        .unwrap_err()
        .contains("handshake differs"));
    assert_eq!(files(&mut app), "first file");
    invoke(&mut app, "core.plugin.untrust", vec!["probe".into()]).unwrap();
    assert_eq!(
        app.plugins()
            .iter()
            .find(|p| p.id == "probe")
            .unwrap()
            .runtime
            .availability,
        Availability::Untrusted
    );
}

#[test]
fn package_boundary_rejects_symlinks_and_uninstall_does_not_reinstall_distribution_default() {
    let fixture = Fixture::new();
    let source = fixture.probe(vec![]);
    std::fs::remove_file(source.join("plugin")).unwrap();
    std::os::unix::fs::symlink(Path::new(env!("CARGO_BIN_EXE_tw")), source.join("plugin")).unwrap();
    assert!(fixture
        .store
        .install(&source)
        .unwrap_err()
        .contains("regular file"));
    fixture.store.uninstall("files").unwrap();
    fixture
        .store
        .bootstrap(
            &FilesPlugin,
            Path::new(env!("CARGO_BIN_EXE_tw")),
            vec!["--serve-files".into()],
        )
        .unwrap();
    assert!(!fixture.store.directory("files").unwrap().exists());
    let package = fixture.base.join("files-again");
    write_package(
        &FilesPlugin,
        Path::new(env!("CARGO_BIN_EXE_tw")),
        vec!["--serve-files".into()],
        &package,
    )
    .unwrap();
    fixture.store.install(&package).unwrap();
    assert!(!fixture.store.is_trusted("files"));
}

#[test]
fn files_preview_reads_beyond_eight_kib_and_reports_oversized_files_without_partial_output() {
    let fixture = Fixture::new();
    let mut app = fixture.app();
    let content = format!("{}\nfinal preview line\n", "строка\n".repeat(2000));
    std::fs::write(fixture.first.join("note.txt"), &content).unwrap();
    assert_eq!(files(&mut app), content);
    // Worst-case JSON escaping still fits inside the executable protocol limit.
    let boundary = "\x01".repeat(128 * 1024);
    std::fs::write(fixture.first.join("note.txt"), &boundary).unwrap();
    assert_eq!(files(&mut app), boundary);
    std::fs::write(fixture.first.join("note.txt"), "x".repeat(128 * 1024 + 1)).unwrap();
    let error = app
        .invoke(CommandInvocation {
            id: "files.preview".into(),
            item: Some("note.txt".into()),
            args: Vec::new(),
        })
        .unwrap_err();
    assert!(error.contains("128 KiB preview limit"));
    std::fs::write(fixture.first.join("note.txt"), "recovered preview").unwrap();
    assert_eq!(files(&mut app), "recovered preview");
}

#[test]
fn old_api_packages_and_text_blocks_remain_compatible() {
    let mut manifest = Manifest::for_plugin(&FilesPlugin, vec!["--serve-files".into()]);
    manifest.api_version = "0.4".into();
    manifest.compatible().unwrap();
    let block: terminal_workspace::Block = serde_json::from_value(
        serde_json::json!({"source":"Old plugin","status":"ok","content":"# literal text"}),
    )
    .unwrap();
    assert_eq!(block.format, terminal_workspace::ContentFormat::Text);
}
