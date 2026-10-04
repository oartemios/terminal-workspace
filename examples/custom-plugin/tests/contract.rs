use console::{strip_ansi_codes, Key};
use terminal_workspace::{ui::Ui, App, CommandInvocation, CommandOutcome, PLUGIN_API_VERSION};
use tw_example_catalog::CatalogPlugin;

#[test]
fn same_custom_plugin_uses_two_workspace_contexts() {
    assert_eq!(PLUGIN_API_VERSION, "0.3");
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
