//! Independently packaged plugin using the same draft API as Files.
use terminal_workspace::{
    Action, BindingScope, Block, Command, CommandInvocation, CommandOutcome, Group, GroupView,
    Item, KeyBinding, Navigation, Plugin, Workspace,
};

pub struct CatalogPlugin;

impl Plugin for CatalogPlugin {
    fn id(&self) -> &'static str {
        "catalog"
    }
    fn name(&self) -> &'static str {
        "Catalog"
    }
    fn groups(&self) -> Vec<Group> {
        vec![Group {
            id: "sections".into(),
            title: "Sections".into(),
        }]
    }
    fn items(&self, workspace: &Workspace, group: &str) -> Result<Vec<Item>, String> {
        Ok(self.view(workspace, group, None)?.items)
    }
    fn view(
        &self,
        workspace: &Workspace,
        group: &str,
        location: Option<&str>,
    ) -> Result<GroupView, String> {
        if group != "sections" {
            return Err("Unknown group".into());
        }
        let location = location.unwrap_or("");
        let (title, item) = match location {
            "" => (
                "Sections",
                Item {
                    id: "intro".into(),
                    title: "Introduction section".into(),
                    kind: "section".into(),
                },
            ),
            "intro" => (
                "Introduction",
                Item {
                    id: "intro.note".into(),
                    title: "Welcome note".into(),
                    kind: "note".into(),
                },
            ),
            _ => return Err("Unknown section".into()),
        };
        let parent = CommandInvocation {
            id: "catalog.parent".into(),
            item: None,
            args: Vec::new(),
        };
        Ok(GroupView {
            title: format!("{title} · {}", workspace.root().display()),
            location: location.into(),
            items: vec![item],
            parent: if location.is_empty() {
                None
            } else {
                Some(parent.clone())
            },
            command_defaults: vec![parent],
        })
    }
    fn actions(&self, item: &Item) -> Vec<Action> {
        let (id, title, is_default) = if item.id == "intro" {
            ("catalog.open", "Open section", true)
        } else {
            ("catalog.read", "Read note", false)
        };
        vec![Action {
            label: title.into(),
            command_id: id.into(),
            is_default,
            invocation: CommandInvocation {
                id: id.into(),
                item: Some(item.id.clone()),
                args: Vec::new(),
            },
        }]
    }
    fn commands(&self) -> Vec<Command> {
        vec![
            Command {
                id: "catalog.open".into(),
                title: "Open section".into(),
                requires_item: true,
            },
            Command {
                id: "catalog.parent".into(),
                title: "Section index".into(),
                requires_item: false,
            },
            Command {
                id: "catalog.read".into(),
                title: "Read note".into(),
                requires_item: true,
            },
        ]
    }
    fn execute(
        &self,
        workspace: &Workspace,
        invocation: &CommandInvocation,
    ) -> Result<CommandOutcome, String> {
        match invocation.id.as_str() {
            "catalog.open" if invocation.item.as_deref() == Some("intro") => {
                Ok(CommandOutcome::Navigate(Navigation {
                    group: "sections".into(),
                    location: "intro".into(),
                    selected: None,
                }))
            }
            "catalog.parent" => Ok(CommandOutcome::Navigate(Navigation {
                group: "sections".into(),
                location: String::new(),
                selected: Some("intro".into()),
            })),
            "catalog.read" if invocation.item.as_deref() == Some("intro.note") => {
                Ok(CommandOutcome::Output(Block {
                    source: "Catalog".into(),
                    status: "ok".into(),
                    content: format!(
                        "{} {}",
                        workspace
                            .plugin_settings("catalog")
                            .and_then(|settings| settings.get("greeting"))
                            .and_then(|value| value.as_str())
                            .unwrap_or("Welcome to"),
                        workspace.root().display()
                    ),
                }))
            }
            _ => Err("Unknown catalog operation or item".into()),
        }
    }

    fn keybindings(&self) -> Vec<KeyBinding> {
        vec![
            KeyBinding {
                keys: "o".into(),
                command_id: "catalog.open".into(),
                scope: BindingScope::Plugin("catalog".into()),
            },
            KeyBinding {
                keys: "u".into(),
                command_id: "catalog.parent".into(),
                scope: BindingScope::Group {
                    plugin: "catalog".into(),
                    group: "sections".into(),
                },
            },
            KeyBinding {
                keys: "nrd".into(),
                command_id: "catalog.read".into(),
                scope: BindingScope::View {
                    plugin: "catalog".into(),
                    group: "sections".into(),
                    location: "intro".into(),
                },
            },
        ]
    }
}
