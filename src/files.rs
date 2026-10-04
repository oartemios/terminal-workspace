use crate::{
    Action, BindingScope, Block, Command, CommandInvocation, CommandOutcome, Group, GroupView,
    Item, KeyBinding, Navigation, Permission, Plugin, Workspace,
};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

pub struct FilesPlugin;

impl Plugin for FilesPlugin {
    fn id(&self) -> &'static str {
        "files"
    }
    fn name(&self) -> &'static str {
        "Files"
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![Permission::WorkspaceRead]
    }

    fn groups(&self) -> Vec<Group> {
        vec![Group {
            id: "entries".into(),
            title: "Entries".into(),
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
        if group != "entries" {
            return Err(format!("Unknown group: {group}"));
        }
        let location = normalize_location(workspace, location.unwrap_or("."))?;
        let directory = workspace.read_path(&location)?;
        if !directory.is_dir() {
            return Err("Location is not a directory".into());
        }
        let mut items = Vec::new();
        for entry in std::fs::read_dir(&directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let id = if location == "." {
                name.clone()
            } else {
                format!("{location}/{name}")
            };
            let file_type = entry.file_type().map_err(|error| error.to_string())?;
            let is_directory = file_type.is_dir()
                || (file_type.is_symlink()
                    && workspace.read_path(&id).is_ok_and(|path| path.is_dir()));
            items.push(Item {
                id,
                title: if is_directory {
                    format!("{name}/")
                } else {
                    name
                },
                kind: if is_directory {
                    "directory".into()
                } else {
                    "file".into()
                },
            });
        }
        items.sort_by(|a, b| a.id.cmp(&b.id));
        let parent = CommandInvocation {
            id: "files.parent".into(),
            item: Some(location.clone()),
            args: Vec::new(),
        };
        Ok(GroupView {
            title: if location == "." {
                "/".into()
            } else {
                format!("/{location}/")
            },
            parent: if location == "." {
                None
            } else {
                Some(parent.clone())
            },
            command_defaults: vec![parent],
            location,
            items,
        })
    }

    fn item_icon(&self, item: &Item) -> char {
        if item.kind == "directory" {
            '▸'
        } else {
            '▤'
        }
    }

    fn actions(&self, item: &Item) -> Vec<Action> {
        if item.kind == "directory" {
            return vec![
                action("open", "Open directory", &item.id),
                action("path", "Show path", &item.id),
            ];
        }
        vec![
            action("path", "Show path", &item.id),
            action("preview", "Preview file", &item.id),
        ]
    }

    fn commands(&self) -> Vec<Command> {
        vec![
            Command {
                id: "files.open".into(),
                title: "Open directory".into(),
                requires_item: true,
            },
            Command {
                id: "files.parent".into(),
                title: "Parent directory".into(),
                requires_item: true,
            },
            Command {
                id: "files.path".into(),
                title: "Show file path".into(),
                requires_item: true,
            },
            Command {
                id: "files.preview".into(),
                title: "Preview file".into(),
                requires_item: true,
            },
        ]
    }

    fn keybindings(&self) -> Vec<KeyBinding> {
        vec![KeyBinding {
            keys: "p".into(),
            command_id: "files.preview".into(),
            scope: BindingScope::Plugin("files".into()),
        }]
    }

    fn execute(
        &self,
        workspace: &Workspace,
        invocation: &CommandInvocation,
    ) -> Result<CommandOutcome, String> {
        let item = invocation.item.as_deref().ok_or("Missing item")?;
        if invocation.id == "files.parent" {
            let current = normalize_location(workspace, item)?;
            if let Ok(path) = workspace.read_path(&current) {
                if !path.is_dir() {
                    return Err(format!("Not a directory: {item}"));
                }
            }
            let parent = if current == "." {
                ".".into()
            } else {
                normalize_location(
                    workspace,
                    Path::new(&current)
                        .parent()
                        .and_then(Path::to_str)
                        .unwrap_or("."),
                )?
            };
            // Resolve the parent independently so deletion of the current folder
            // does not trap navigation, and a symlink cannot lead outside root.
            if !workspace.read_path(&parent)?.is_dir() {
                return Err("Parent is not a directory".into());
            }
            return Ok(CommandOutcome::Navigate(Navigation {
                group: "entries".into(),
                location: parent,
                selected: if current == "." { None } else { Some(current) },
            }));
        }
        if invocation.id == "files.open" {
            let location = normalize_location(workspace, item)?;
            if !workspace.read_path(&location)?.is_dir() {
                return Err(format!("Not a directory: {item}"));
            }
            return Ok(CommandOutcome::Navigate(Navigation {
                group: "entries".into(),
                location,
                selected: None,
            }));
        }
        let path = workspace.read_path(item)?;
        let content = match invocation.id.as_str() {
            "files.path" => path.display().to_string(),
            "files.preview" => {
                if !path.is_file() {
                    return Err(format!("Not a file: {item}"));
                }
                let mut bytes = Vec::new();
                std::fs::File::open(&path)
                    .map_err(|error| error.to_string())?
                    .take(8192)
                    .read_to_end(&mut bytes)
                    .map_err(|error| error.to_string())?;
                String::from_utf8_lossy(&bytes).into_owned()
            }
            _ => return Err(format!("Unknown Files command: {}", invocation.id)),
        };
        Ok(CommandOutcome::Output(Block {
            source: "Files".into(),
            status: "ok".into(),
            content,
        }))
    }
}

fn action(name: &str, label: &str, item: &str) -> Action {
    let id = format!("files.{name}");
    Action {
        is_default: name == "open",
        label: label.into(),
        command_id: id.clone(),
        invocation: CommandInvocation {
            id,
            item: Some(item.into()),
            args: Vec::new(),
        },
    }
}

fn normalize_location(workspace: &Workspace, location: &str) -> Result<String, String> {
    let path = Path::new(location);
    let relative = if path.is_absolute() {
        path.strip_prefix(workspace.root())
            .map_err(|_| "Path is outside the workspace")?
    } else {
        path
    };
    let mut normalized = PathBuf::new();
    for component in relative.components() {
        match component {
            Component::Normal(name) => normalized.push(name),
            Component::CurDir => {}
            Component::ParentDir if normalized.pop() => {}
            _ => return Err("Path is outside the workspace".into()),
        }
    }
    if normalized.as_os_str().is_empty() {
        Ok(".".into())
    } else {
        normalized
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| "Path is not valid UTF-8".into())
    }
}
