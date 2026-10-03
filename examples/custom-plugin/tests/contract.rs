use console::{strip_ansi_codes, Key};
use terminal_workspace::{ui::Ui, App, CommandInvocation, CommandOutcome, PLUGIN_API_VERSION};
use tw_example_catalog::CatalogPlugin;

#[test]
fn same_custom_plugin_uses_two_workspace_contexts() {
    assert_eq!(PLUGIN_API_VERSION, "0.1");
    for root in [std::env::current_dir().unwrap(), std::env::temp_dir()] {
        let mut app = App::new(root).unwrap();
        app.install(Box::new(CatalogPlugin), true).unwrap();
        let result = app
            .invoke(CommandInvocation {
                id: "catalog.open".into(),
                item: Some("intro".into()),
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
        assert!(strip_ansi_codes(&ui.render(120, 24)).contains("> Introduction section"));
    }
}
