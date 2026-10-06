//! Local executable packages and JSON-lines protocol. Process containment is not an OS sandbox.
mod background;
mod package;
mod process;
mod server;

pub(crate) use background::run as background_run;
pub use background::{BackgroundRequest, BackgroundResponse};
pub use package::{write_package, Manifest, PackageStore};
pub use process::ProcessPlugin;
pub use server::serve_plugin;

use crate::{
    Command, CommandInvocation, Group, KeyBinding, Permission, Plugin, RefreshStrategy, Workspace,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_MESSAGE: usize = 1_048_576;
pub const REQUEST_TIMEOUT_MS: u64 = 1000;
pub const STARTUP_TIMEOUT_MS: u64 = 5000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Descriptor {
    pub id: String,
    pub name: String,
    pub groups: Vec<Group>,
    pub commands: Vec<Command>,
    #[serde(default)]
    pub keybindings: Vec<KeyBinding>,
    #[serde(default)]
    pub permissions: Vec<Permission>,
    /// Non-manual strategies only; absent in API <= 0.6 manifests.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refresh: Vec<(String, RefreshStrategy)>,
}

impl Descriptor {
    pub fn from_plugin(plugin: &dyn Plugin) -> Self {
        let groups = plugin.groups();
        let refresh = groups
            .iter()
            .filter_map(|group| {
                let strategy = plugin.refresh_strategy(&group.id);
                (strategy != RefreshStrategy::Manual).then_some((group.id.clone(), strategy))
            })
            .collect();
        Self {
            id: plugin.id().into(),
            name: plugin.name().into(),
            groups,
            commands: plugin.commands(),
            keybindings: plugin.keybindings(),
            permissions: plugin.permissions(),
            refresh,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.id == "core"
            || self.id.is_empty()
            || self.id.len() > 64
            || !self
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err("Package plugin id must be a non-reserved ASCII name".into());
        }
        if self.groups.len() > 256 || self.commands.len() > 512 || self.keybindings.len() > 1024 {
            return Err("Plugin declaration exceeds limits".into());
        }
        let mut groups = BTreeSet::new();
        for group in &self.groups {
            if group.id.is_empty() || !groups.insert(&group.id) {
                return Err("Invalid or duplicate group".into());
            }
        }
        let mut refresh_groups = BTreeSet::new();
        for (group, strategy) in &self.refresh {
            if !groups.contains(group) || !refresh_groups.insert(group) {
                return Err("Refresh strategy refers to an unknown or duplicate group".into());
            }
            if matches!(strategy, RefreshStrategy::Interval { seconds: 0 }) {
                return Err("Refresh interval must be at least one second".into());
            }
        }
        let mut commands = BTreeSet::new();
        for command in &self.commands {
            if !command.id.starts_with(&format!("{}.", self.id)) || !commands.insert(&command.id) {
                return Err(format!("Invalid or duplicate command: {}", command.id));
            }
        }
        for binding in &self.keybindings {
            crate::bindings::validate_keys(&binding.keys, &binding.scope)?;
            if binding.scope.plugin() != Some(self.id.as_str())
                || !commands.contains(&binding.command_id)
            {
                return Err(format!("Invalid binding: {}", binding.keys));
            }
            match &binding.scope {
                crate::BindingScope::Group { group, .. }
                | crate::BindingScope::View { group, .. }
                    if !groups.contains(group) =>
                {
                    return Err("Binding refers to an undeclared group".into())
                }
                _ => {}
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    Available,
    Untrusted,
    Incompatible,
    Missing,
    Failed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connection {
    InProcess,
    Disconnected,
    Running,
    Loading,
    Failed,
}

#[derive(Clone, Debug)]
pub struct RuntimeStatus {
    pub availability: Availability,
    pub connection: Connection,
    pub detail: Option<String>,
}
impl RuntimeStatus {
    pub fn in_process() -> Self {
        Self {
            availability: Availability::Available,
            connection: Connection::InProcess,
            detail: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Context {
    pub root: PathBuf,
    pub settings: Value,
    pub permissions: BTreeSet<Permission>,
}
impl Context {
    pub fn new(workspace: &Workspace, plugin: &str, permissions: &BTreeSet<Permission>) -> Self {
        Self {
            root: workspace.root().into(),
            settings: workspace
                .plugin_settings(plugin)
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
            permissions: permissions.clone(),
        }
    }
    pub fn workspace(&self, plugin: &str) -> Result<Workspace, String> {
        if !self.root.is_absolute() || !self.settings.is_object() {
            return Err("Invalid workspace context".into());
        }
        // Do not canonicalize or enumerate files until an authorized plugin call asks for them.
        Ok(Workspace {
            root: self.root.clone(),
            config: serde_json::json!({"version":1,"plugins":{plugin:{"settings":self.settings}},"overrides":{}}),
            runtime_permissions: Some(self.permissions.clone()),
        })
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Request {
    pub protocol: u32,
    pub request_id: u64,
    #[serde(flatten)]
    pub body: Operation,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Operation {
    Describe,
    View {
        context: Context,
        group: String,
        location: Option<String>,
    },
    Refresh {
        context: Context,
        group: String,
        location: Option<String>,
    },
    Actions {
        context: Context,
        item: crate::Item,
    },
    Execute {
        context: Context,
        invocation: CommandInvocation,
    },
}
#[derive(Serialize, Deserialize)]
pub(crate) struct Reply {
    pub protocol: u32,
    pub request_id: u64,
    pub result: Result<Value, String>,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct ViewReply {
    pub view: crate::GroupView,
    pub icons: std::collections::BTreeMap<String, char>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn worker_context_filters_settings_and_checks_read_write_path_helpers() {
        let base = std::env::temp_dir().join(format!("tw-context-{}", std::process::id()));
        std::fs::create_dir(&base).unwrap();
        let root = base.join("project");
        let outside = base.join("outside");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
        let mut original = Workspace::open(root).unwrap();
        original.config = serde_json::json!({"plugins":{"probe":{"settings":{"greeting":"own"}},"other":{"settings":{"token":"not sent"}}},"overrides":{}});
        let context = Context::new(
            &original,
            "probe",
            &BTreeSet::from([Permission::WorkspaceWrite]),
        );
        let worker = context.workspace("probe").unwrap();
        assert!(worker.plugin_settings("other").is_none());
        assert_eq!(worker.plugin_settings("probe").unwrap()["greeting"], "own");
        std::fs::write(worker.write_path("new.txt").unwrap(), "authorized write").unwrap();
        assert!(worker
            .read_path("new.txt")
            .unwrap_err()
            .contains("WorkspaceRead"));
        for path in ["../outside/file", "escape/file", "/etc/hosts"] {
            assert!(worker.write_path(path).is_err());
        }
        let read_only = Context::new(
            &original,
            "probe",
            &BTreeSet::from([Permission::WorkspaceRead]),
        )
        .workspace("probe")
        .unwrap();
        assert!(read_only.read_path("new.txt").is_ok());
        assert!(read_only
            .write_path("new.txt")
            .unwrap_err()
            .contains("WorkspaceWrite"));
        std::fs::remove_dir_all(base).unwrap();
    }
}
