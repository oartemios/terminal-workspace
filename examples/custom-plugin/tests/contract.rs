use console::{strip_ansi_codes, Key};
use terminal_workspace::{ui::Ui, App, CommandInvocation, CommandOutcome, PLUGIN_API_VERSION};
use tw_example_catalog::CatalogPlugin;

#[test]
fn same_custom_plugin_uses_two_workspace_contexts() {
    assert_eq!(PLUGIN_API_VERSION, "0.4");
    for root in [std::env::current_dir().unwrap(), std::env::temp_dir()] {
        let mut app = App::new(root).unwrap();
        app.install(Box::new(CatalogPlugin), true).unwrap();
        let result = app
            .invoke(CommandInvocation {
                id: "catalog.open".into(),
                item: Some("intro".into()),
                args: Vec::new(),
            })
            .unwrap();
        let CommandOutcome::Navigate(navigation) = result else {
            panic!("Expected navigation");
        };
        let view = app
            .view("catalog", &navigation.group, Some(&navigation.location))
            .unwrap();
        assert_eq!(view.items[0].id, "intro.note");
        let expected = format!("Welcome to {}", app.workspace().root().display());
        let result = app
            .invoke(
                app.actions_at("catalog", "sections", Some("intro"), "intro.note")
                    .unwrap()[0]
                    .invocation
                    .clone(),
            )
            .unwrap();
        let CommandOutcome::Output(block) = result else {
            panic!("Expected output");
        };
        assert_eq!(block.content, expected);
        let mut ui = Ui::new(app);
        ui.handle(Key::Enter);
        assert!(strip_ansi_codes(&ui.render(120, 24)).contains("Welcome note"));
        ui.handle(Key::Enter);
        ui.handle(Key::Enter);
        assert!(strip_ansi_codes(&ui.render(120, 24)).contains("Welcome to"));
        ui.handle(Key::Escape);
        ui.handle(Key::Backspace);
        assert!(strip_ansi_codes(&ui.render(120, 24)).contains("> • Introduction section"));
    }
}

#[test]
fn same_custom_plugin_receives_persistent_settings_and_activation_in_two_projects() {
    let roots: Vec<_> = ["Hello", "Greetings"].iter().enumerate().map(|(index, greeting)| {
        let root = std::env::temp_dir().join(format!("tw-custom-config-{}-{index}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join(terminal_workspace::CONFIG_FILE), format!(r#"{{"version":1,"plugins":{{"catalog":{{"enabled":true,"settings":{{"greeting":"{greeting}"}}}}}}}}"#)).unwrap();
        (root, *greeting)
    }).collect();
    for (root, greeting) in &roots {
        let mut app = App::open(root.clone()).unwrap();
        app.install(Box::new(CatalogPlugin), false).unwrap();
        let result = app
            .invoke(CommandInvocation {
                id: "catalog.read".into(),
                item: Some("intro.note".into()),
                args: Vec::new(),
            })
            .unwrap();
        let CommandOutcome::Output(block) = result else {
            panic!("Expected output");
        };
        assert_eq!(
            block.content,
            format!("{greeting} {}", app.workspace().root().display())
        );
        app.disable("catalog").unwrap();
        let mut restarted = App::open(root.clone()).unwrap();
        restarted.install(Box::new(CatalogPlugin), true).unwrap();
        assert_eq!(restarted.plugins()[0].status, "workspace disabled");
        restarted.enable("catalog").unwrap();
        assert_eq!(
            restarted.workspace().plugin_settings("catalog").unwrap()["greeting"],
            *greeting
        );
    }
    for (root, _) in roots {
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn installed_custom_plugin_switches_projects_and_all_routes_use_current_settings() {
    let roots: Vec<_> = ["First greeting", "Second greeting"].iter().enumerate().map(|(i,greeting)| {
        let root = std::env::temp_dir().join(format!("tw-custom-switch-{}-{i}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join(terminal_workspace::CONFIG_FILE), serde_json::json!({"version":1,"plugins":{"catalog":{"enabled":true,"settings":{"greeting":greeting}}},"overrides":{}}).to_string()).unwrap();
        root
    }).collect();
    let mut app = App::open(roots[0].clone()).unwrap();
    app.install(Box::new(CatalogPlugin), false).unwrap();
    let mut ui = Ui::new(app);
    fn text(ui: &mut Ui, value: &str) {
        for c in value.chars() {
            assert!(ui.handle(Key::Char(c)));
        }
    }
    fn frame(ui: &Ui) -> String {
        strip_ansi_codes(&ui.render(180, 32)).into_owned()
    }
    // Opening and returning use the same UI navigation as Files.
    text(&mut ui, "l");
    ui.resize_to(180, 32);
    ui.handle(Key::Char('?'));
    ui.handle(Key::End);
    assert!(frame(&ui).contains("catalog.read"), "{}", frame(&ui));
    ui.handle(Key::Escape);
    text(&mut ui, "p");
    assert!(frame(&ui).contains("First greeting"));
    ui.handle(Key::Escape);
    text(
        &mut ui,
        &format!(":core.workspace.open {}", roots[1].display()),
    );
    ui.handle(Key::Enter);
    assert!(frame(&ui).contains("Introduction section"));
    assert!(!frame(&ui).contains("Welcome note"));
    text(&mut ui, "l");
    for route in 0..4 {
        match route {
            0 => text(&mut ui, "p"),
            1 => {
                text(&mut ui, ":catalog.read");
                ui.handle(Key::Enter);
            }
            2 => {
                text(&mut ui, " catalog.read");
                ui.handle(Key::Enter);
            }
            _ => {
                text(&mut ui, "a");
                ui.handle(Key::Enter);
            }
        }
        assert!(
            frame(&ui).contains("Second greeting"),
            "route {route}: {}",
            frame(&ui)
        );
        assert!(!frame(&ui).contains("First greeting"));
        ui.handle(Key::Escape);
    }
    text(&mut ui, ",b");
    assert!(frame(&ui).contains("Welcome note"));
    text(&mut ui, "p");
    assert!(frame(&ui).contains("First greeting"));
    ui.handle(Key::Escape);
    text(&mut ui, "h");
    assert!(frame(&ui).contains("> • Introduction section"));
    // View-scoped bindings disappear when leaving that view.
    text(&mut ui, "p");
    assert!(!frame(&ui).contains("First greeting"));
    for root in roots {
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn independently_built_package_installs_and_runs_through_the_keyboard_ui() {
    use terminal_workspace::runtime::{write_package, PackageStore};
    let base = std::env::temp_dir().join(format!("tw-custom-package-{}", std::process::id()));
    std::fs::create_dir(&base).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(base.clone());
    let first = base.join("first project");
    let second = base.join("second project");
    for (root, greeting, enabled) in [
        (&first, "First runtime greeting", false),
        (&second, "Second runtime greeting", true),
    ] {
        std::fs::create_dir(root).unwrap();
        std::fs::write(root.join(terminal_workspace::CONFIG_FILE),serde_json::json!({"version":1,"plugins":{"catalog":{"enabled":enabled,"settings":{"greeting":greeting}}},"overrides":{}}).to_string()).unwrap();
    }
    let source = base.join("catalog package");
    write_package(
        &CatalogPlugin,
        std::path::Path::new(env!("CARGO_BIN_EXE_tw-example-catalog")),
        Vec::new(),
        &source,
    )
    .unwrap();
    let store = PackageStore::new(base.join("global plugins")).unwrap();
    let mut app = App::open(first.clone()).unwrap();
    assert!(app.load_packages(store.clone(), &[]).is_empty());
    let mut ui = Ui::new(app);
    ui.resize_to(180, 32);
    fn text(ui: &mut Ui, value: &str) {
        for c in value.chars() {
            assert!(ui.handle(Key::Char(c)));
        }
    }
    fn frame(ui: &Ui) -> String {
        strip_ansi_codes(&ui.render(180, 32)).into_owned()
    }
    fn command(ui: &mut Ui, value: &str) {
        text(ui, &format!(":{value}"));
        ui.handle(Key::Enter);
    }
    text(&mut ui, " core.plugin.install");
    ui.handle(Key::Enter);
    assert!(frame(&ui).contains(":core.plugin.install "));
    text(&mut ui, &source.to_string_lossy());
    ui.handle(Key::Enter);
    assert!(frame(&ui).contains("catalog: installed"));
    ui.handle(Key::Escape);
    command(&mut ui, "core.plugin.enable catalog");
    ui.handle(Key::Escape);
    assert!(frame(&ui).contains("untrusted"));
    command(&mut ui, "core.plugin.trust catalog");
    ui.handle(Key::Escape);
    assert!(frame(&ui).contains("Introduction section"));
    text(&mut ui, "l");
    for route in 0..4 {
        match route {
            0 => text(&mut ui, "p"),
            1 => command(&mut ui, "catalog.read"),
            2 => {
                text(&mut ui, " catalog.read");
                ui.handle(Key::Enter);
            }
            _ => {
                text(&mut ui, "a");
                ui.handle(Key::Enter);
            }
        }
        assert!(
            frame(&ui).contains("First runtime greeting"),
            "route {route}: {}",
            frame(&ui)
        );
        ui.handle(Key::Escape);
    }
    command(
        &mut ui,
        &format!("core.workspace.open {}", second.display()),
    );
    text(&mut ui, "lp");
    assert!(frame(&ui).contains("Second runtime greeting"));
    ui.handle(Key::Escape);
    text(&mut ui, ",b");
    text(&mut ui, "p");
    assert!(frame(&ui).contains("First runtime greeting"));
    ui.handle(Key::Escape);
    text(&mut ui, ",s");
    assert!(frame(&ui).contains("session disabled"));
    ui.handle(Key::Escape);
    text(&mut ui, ",e");
    ui.handle(Key::Escape);
    assert!(frame(&ui).contains("Introduction section"));
    let saved = std::fs::read(first.join(terminal_workspace::CONFIG_FILE)).unwrap();
    command(&mut ui, "core.plugin.uninstall catalog");
    assert!(frame(&ui).contains("catalog: uninstalled"));
    ui.handle(Key::Escape);
    assert_eq!(
        std::fs::read(first.join(terminal_workspace::CONFIG_FILE)).unwrap(),
        saved
    );
    command(&mut ui, "core.plugins");
    assert!(frame(&ui).contains("No installed plugins"));
    assert!(!store.directory("catalog").unwrap().exists());
}
