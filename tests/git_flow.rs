use console::{strip_ansi_codes, Key};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use terminal_workspace::{
    files::FilesPlugin,
    git::GitPlugin,
    runtime::{write_package, PackageStore},
    ui::Ui,
    App, CommandInvocation, CommandOutcome, Permission,
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    base: PathBuf,
    first: PathBuf,
    second: PathBuf,
    store: PackageStore,
}
fn git(root: &Path, args: &[&str]) {
    let result = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{:?}: {}",
        args,
        String::from_utf8_lossy(&result.stderr)
    );
}
impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "tw-git-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&base).unwrap();
        let base = base.canonicalize().unwrap();
        let first = base.join("first");
        let second = base.join("second");
        for root in [&first, &second] {
            std::fs::create_dir(root).unwrap();
            git(root, &["init", "-b", "main"]);
            git(root, &["config", "user.name", "Fixture"]);
            git(root, &["config", "user.email", "fixture@example.test"]);
        }
        let store = PackageStore::new(base.join("plugins")).unwrap();
        let package = base.join("package");
        write_package(
            &GitPlugin,
            Path::new(env!("CARGO_BIN_EXE_tw")),
            vec!["--serve-git".into()],
            &package,
        )
        .unwrap();
        store.install(&package).unwrap();
        store.trust("git").unwrap();
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
    fn app(&self, root: &Path) -> App {
        let mut app = App::open(root.into()).unwrap();
        assert!(app.load_packages(self.store.clone(), &["files"]).is_empty());
        app.grant("files", Permission::WorkspaceRead).unwrap();
        app.enable("git").unwrap();
        app.grant("git", Permission::WorkspaceRead).unwrap();
        app.grant("git", Permission::Process).unwrap();
        app
    }
    fn commit(root: &Path, path: &str, content: &str) {
        std::fs::write(root.join(path), content).unwrap();
        git(root, &["add", "--", path]);
        git(root, &["commit", "-m", "initial"]);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}
fn output(app: &mut App, command: &str, item: &str) -> Result<String, String> {
    match app.invoke(CommandInvocation {
        id: command.into(),
        item: Some(item.into()),
        args: vec![],
    })? {
        CommandOutcome::Output(block) => {
            assert_eq!(block.source, "Git");
            Ok(block.content)
        }
        _ => panic!("Expected Git Block"),
    }
}
fn text(ui: &mut Ui, input: &str) {
    for c in input.chars() {
        assert!(ui.handle(Key::Char(c)));
    }
}
fn frame(ui: &Ui) -> String {
    strip_ansi_codes(&ui.render(140, 32)).into_owned()
}
fn git_ui(app: App) -> Ui {
    let mut ui = Ui::new(app);
    ui.resize_to(140, 32);
    ui.handle(Key::Tab);
    ui.handle(Key::ArrowRight);
    ui.handle(Key::Enter);
    ui.handle(Key::Enter);
    assert!(frame(&ui).contains("[git]"));
    ui
}
#[test]
fn packaged_git_handles_staged_unstaged_renames_deleted_and_literal_paths() {
    let f = Fixture::new();
    Fixture::commit(&f.first, "note with spaces.md", "original\n");
    Fixture::commit(&f.first, "old.txt", "rename me\n");
    Fixture::commit(&f.first, "deleted.txt", "delete me\n");
    std::fs::write(f.first.join("note with spaces.md"), "staged\n").unwrap();
    git(&f.first, &["add", "--", "note with spaces.md"]);
    std::fs::write(f.first.join("note with spaces.md"), "working\n").unwrap();
    git(&f.first, &["mv", "old.txt", "новое имя.txt"]);
    std::fs::remove_file(f.first.join("deleted.txt")).unwrap();
    for path in [":(glob)*", "--option.txt", "line\nbreak.txt"] {
        std::fs::write(f.first.join(path), "untracked content\n").unwrap();
    }
    let index = std::fs::read(f.first.join(".git/index")).unwrap();
    let head = std::fs::read(f.first.join(".git/refs/heads/main")).unwrap();
    let mut app = f.app(&f.first);
    let items = app.items("git", "status").unwrap();
    assert!(items.iter().any(|i| i.title == "MM note with spaces.md"));
    assert!(items
        .iter()
        .any(|i| i.title.contains("old.txt → новое имя.txt")));
    let diff = output(&mut app, "git.diff", "note with spaces.md").unwrap();
    assert!(diff.contains("Staged (HEAD → index)"));
    assert!(diff.contains("+staged"));
    assert!(diff.contains("Unstaged (index → working tree)"));
    assert!(diff.contains("+working"));
    assert!(output(&mut app, "git.diff", "новое имя.txt")
        .unwrap()
        .contains("rename from old.txt"));
    assert!(output(&mut app, "git.diff", "deleted.txt")
        .unwrap()
        .contains("-delete me"));
    for path in [":(glob)*", "--option.txt", "line\nbreak.txt"] {
        assert!(output(&mut app, "git.diff", path)
            .unwrap()
            .contains("+untracked content"));
    }
    assert!(output(&mut app, "git.diff", "../secret").is_err());
    assert!(output(&mut app, "git.diff", ".git/config").is_err());
    assert_eq!(std::fs::read(f.first.join(".git/index")).unwrap(), index);
    assert_eq!(
        std::fs::read(f.first.join(".git/refs/heads/main")).unwrap(),
        head
    );
    assert_eq!(
        std::fs::read_to_string(f.first.join("note with spaces.md")).unwrap(),
        "working\n"
    );
}
#[test]
fn all_entry_points_use_git_commands_and_group_scoped_bindings() {
    let f = Fixture::new();
    Fixture::commit(&f.first, "note with spaces.md", "before\n");
    std::fs::write(f.first.join("note with spaces.md"), "after\n").unwrap();
    for route in 0..4 {
        let mut ui = git_ui(f.app(&f.first));
        text(&mut ui, "/note with spaces");
        ui.handle(Key::Enter);
        match route {
            0 => {
                ui.handle(Key::Char('a'));
                assert!(frame(&ui).contains("View file diff"));
                ui.handle(Key::Enter);
            }
            1 => text(&mut ui, "p"),
            2 => {
                text(&mut ui, ":git.diff note with spaces.md");
                ui.handle(Key::Enter);
            }
            _ => {
                text(&mut ui, " git.diff");
                ui.handle(Key::Enter);
            }
        }
        assert!(
            frame(&ui).contains("+after"),
            "route {route}: {}",
            frame(&ui)
        );
        ui.handle(Key::Escape);
        assert!(frame(&ui).contains("note with spaces.md"));
        ui.handle(Key::BackTab);
        ui.handle(Key::ArrowRight);
        ui.handle(Key::Enter);
        assert!(frame(&ui).contains("[Branches]"));
        text(&mut ui, "p");
        assert!(frame(&ui).contains("Branch: main"));
        ui.handle(Key::Escape);
        assert!(frame(&ui).contains("* main"));
    }
}
#[test]
fn two_workspaces_empty_unborn_and_detached_branches() {
    let f = Fixture::new();
    let mut app = f.app(&f.first);
    assert!(app.items("git", "branches").unwrap().is_empty());
    // App config is itself untracked; keep it out of Status to test a clean repo.
    git(&f.first, &["config", "status.showUntrackedFiles", "no"]);
    Fixture::commit(&f.first, ".gitignore", ".terminal-workspace.json\n");
    assert!(app.items("git", "status").unwrap().is_empty());
    Fixture::commit(&f.first, "first.txt", "first\n");
    Fixture::commit(&f.second, "second.txt", "second\n");
    git(&f.second, &["branch", "topic"]);
    app.switch_workspace(f.second.clone()).unwrap();
    // Workspace activation/permissions do not propagate from the first project.
    assert!(app.items("git", "branches").is_err());
    app.enable("git").unwrap();
    app.grant("git", Permission::WorkspaceRead).unwrap();
    app.grant("git", Permission::Process).unwrap();
    assert!(app
        .items("git", "branches")
        .unwrap()
        .iter()
        .any(|i| i.id == "refs/heads/topic"));
    assert!(output(&mut app, "git.branch", "refs/heads/main")
        .unwrap()
        .contains("initial"));
    git(&f.second, &["checkout", "--detach"]);
    assert!(app
        .items("git", "branches")
        .unwrap()
        .iter()
        .all(|i| !i.title.starts_with('*')));
    app.switch_workspace(f.first.clone()).unwrap();
    assert_eq!(app.items("git", "branches").unwrap().len(), 1);
}
#[test]
fn permissions_errors_and_lifecycle_leave_files_available() {
    let f = Fixture::new();
    std::fs::write(f.first.join("local.txt"), "local content").unwrap();
    let mut app = f.app(&f.first);
    app.revoke("git", Permission::Process).unwrap();
    assert!(app.items("git", "status").unwrap_err().contains("Process"));
    assert!(app
        .items("files", "entries")
        .unwrap()
        .iter()
        .any(|i| i.id == "local.txt"));
    app.grant("git", Permission::Process).unwrap();
    app.disable_for_session("git").unwrap();
    assert!(app.items("git", "status").is_err());
    app.enable("git").unwrap();
    assert!(app.items("git", "status").is_ok());
    let ordinary = f.base.join("not-a-repo");
    std::fs::create_dir(&ordinary).unwrap();
    let app = f.app(&ordinary);
    assert!(app
        .items("git", "status")
        .unwrap_err()
        .contains("not a git repository"));
    assert!(app.items("files", "entries").is_ok());
}
#[test]
fn oversized_diff_is_a_domain_error_and_worker_recovers() {
    let f = Fixture::new();
    let mut app = f.app(&f.first);
    std::fs::write(f.first.join("large.txt"), "x".repeat(200_000)).unwrap();
    assert!(output(&mut app, "git.diff", "large.txt")
        .unwrap_err()
        .contains("128 KiB"));
    std::fs::write(f.first.join("large.txt"), "small\n").unwrap();
    assert!(output(&mut app, "git.diff", "large.txt")
        .unwrap()
        .contains("+small"));
}
