use std::collections::{BTreeMap, BTreeSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

pub mod files;
pub mod ui;

/// Draft source-level Plugin API; no dynamic ABI or isolation is implied.
pub const PLUGIN_API_VERSION: &str = "0.1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub id: String,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub title: String,
    /// Native kind chosen and interpreted by the owning plugin.
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub id: String,
    pub title: String,
    pub requires_item: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandInvocation {
    pub id: String,
    pub item: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub label: String,
    pub command_id: String,
    pub invocation: CommandInvocation,
    pub is_default: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub source: String,
    pub status: String,
    pub content: String,
}

/// Location and item identifiers are opaque to Core, and interpreted by a plugin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Navigation {
    pub group: String,
    pub location: String,
    pub selected: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandOutcome {
    Output(Block),
    Navigate(Navigation),
}

pub struct GroupView {
    pub title: String,
    pub location: String,
    pub items: Vec<Item>,
    pub parent: Option<CommandInvocation>,
    pub command_defaults: Vec<CommandInvocation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Permission {
    WorkspaceRead,
}

pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    pub fn open(root: PathBuf) -> Result<Self, String> {
        let root = root.canonicalize().map_err(|error| error.to_string())?;
        if !root.is_dir() {
            return Err(format!("Not a directory: {}", root.display()));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn read_path(&self, relative: &str) -> Result<PathBuf, String> {
        let path = self
            .root
            .join(relative)
            .canonicalize()
            .map_err(|error| error.to_string())?;
        if !path.starts_with(&self.root) {
            return Err("Path is outside the workspace".into());
        }
        Ok(path)
    }
}

pub trait Plugin {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn permissions(&self) -> Vec<Permission> {
        Vec::new()
    }
    fn groups(&self) -> Vec<Group>;
    fn items(&self, workspace: &Workspace, group: &str) -> Result<Vec<Item>, String>;
    fn view(
        &self,
        workspace: &Workspace,
        group: &str,
        location: Option<&str>,
    ) -> Result<GroupView, String> {
        if location.is_some_and(|location| !location.is_empty()) {
            return Err("Plugin does not support nested locations".into());
        }
        Ok(GroupView {
            title: group.into(),
            location: String::new(),
            items: self.items(workspace, group)?,
            parent: None,
            command_defaults: Vec::new(),
        })
    }
    fn actions(&self, item: &Item) -> Vec<Action>;
    fn commands(&self) -> Vec<Command>;
    fn execute(
        &self,
        workspace: &Workspace,
        invocation: &CommandInvocation,
    ) -> Result<CommandOutcome, String>;
}

pub struct PluginState {
    pub id: String,
    pub status: &'static str,
}

pub struct App {
    workspace: Workspace,
    installed: BTreeMap<String, Box<dyn Plugin>>,
    workspace_enabled: BTreeSet<String>,
    session_disabled: BTreeSet<String>,
    registry: BTreeMap<String, (String, Command)>,
    requested_permissions: BTreeMap<String, BTreeSet<Permission>>,
    granted_permissions: BTreeMap<String, BTreeSet<Permission>>,
}

impl App {
    pub fn new(root: PathBuf) -> Result<Self, String> {
        Ok(Self {
            workspace: Workspace::open(root)?,
            installed: BTreeMap::new(),
            workspace_enabled: BTreeSet::new(),
            session_disabled: BTreeSet::new(),
            registry: BTreeMap::new(),
            requested_permissions: BTreeMap::new(),
            granted_permissions: BTreeMap::new(),
        })
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    pub fn install(&mut self, plugin: Box<dyn Plugin>, enabled: bool) -> Result<(), String> {
        let id = plugin.id().to_owned();
        if self.installed.contains_key(&id) {
            return Err(format!("Plugin already installed: {id}"));
        }
        let commands = catch_unwind(AssertUnwindSafe(|| plugin.commands()))
            .map_err(|_| format!("Plugin {id} panicked while registering commands"))?;
        let permissions = catch_unwind(AssertUnwindSafe(|| plugin.permissions()))
            .map_err(|_| format!("Plugin {id} panicked while declaring permissions"))?;
        let mut new_ids = BTreeSet::new();
        for command in &commands {
            if !new_ids.insert(command.id.clone())
                || self.registry.contains_key(&command.id)
                || !command.id.starts_with(&format!("{id}."))
            {
                return Err(format!("Invalid or duplicate command: {}", command.id));
            }
        }
        for command in commands {
            self.registry
                .insert(command.id.clone(), (id.clone(), command));
        }
        self.installed.insert(id.clone(), plugin);
        self.requested_permissions
            .insert(id.clone(), permissions.into_iter().collect());
        if enabled {
            self.workspace_enabled.insert(id);
        }
        Ok(())
    }

    pub fn enable(&mut self, id: &str) -> Result<(), String> {
        if !self.installed.contains_key(id) {
            return Err(format!("Plugin not installed: {id}"));
        }
        self.workspace_enabled.insert(id.to_owned());
        self.session_disabled.remove(id);
        Ok(())
    }

    pub fn grant(&mut self, id: &str, permission: Permission) -> Result<(), String> {
        if !self
            .requested_permissions
            .get(id)
            .is_some_and(|requested| requested.contains(&permission))
        {
            return Err(format!("Plugin {id} did not request {permission:?}"));
        }
        self.granted_permissions
            .entry(id.to_owned())
            .or_default()
            .insert(permission);
        Ok(())
    }

    fn check_permissions(&self, id: &str) -> Result<(), String> {
        let requested = self
            .requested_permissions
            .get(id)
            .ok_or_else(|| format!("Plugin not installed: {id}"))?;
        let granted = self.granted_permissions.get(id);
        for permission in requested {
            if !granted.is_some_and(|granted| granted.contains(permission)) {
                return Err(format!("Permission {permission:?} not granted to {id}"));
            }
        }
        Ok(())
    }

    pub fn disable(&mut self, id: &str) {
        self.workspace_enabled.remove(id);
    }
    pub fn disable_for_session(&mut self, id: &str) {
        self.session_disabled.insert(id.to_owned());
    }

    pub fn plugins(&self) -> Vec<PluginState> {
        self.installed
            .keys()
            .map(|id| PluginState {
                id: id.clone(),
                status: if !self.workspace_enabled.contains(id) {
                    "workspace disabled"
                } else if self.session_disabled.contains(id) {
                    "session disabled"
                } else {
                    "active"
                },
            })
            .collect()
    }

    fn active(&self, id: &str) -> Result<&dyn Plugin, String> {
        let plugin = self
            .installed
            .get(id)
            .ok_or_else(|| format!("Plugin not installed: {id}"))?;
        if !self.workspace_enabled.contains(id) || self.session_disabled.contains(id) {
            return Err(format!("Plugin inactive: {id}"));
        }
        Ok(plugin.as_ref())
    }

    pub fn groups(&self, id: &str) -> Result<Vec<Group>, String> {
        let plugin = self.active(id)?;
        catch_unwind(AssertUnwindSafe(|| plugin.groups()))
            .map_err(|_| format!("Plugin {id} failed while listing groups"))
    }

    pub fn items(&self, id: &str, group: &str) -> Result<Vec<Item>, String> {
        Ok(self.view(id, group, None)?.items)
    }

    pub fn view(&self, id: &str, group: &str, location: Option<&str>) -> Result<GroupView, String> {
        let plugin = self.active(id)?;
        self.check_permissions(id)?;
        catch_unwind(AssertUnwindSafe(|| {
            plugin.view(&self.workspace, group, location)
        }))
        .map_err(|_| format!("Plugin {id} failed while loading items"))?
    }

    pub fn actions(&self, id: &str, group: &str, item_id: &str) -> Result<Vec<Action>, String> {
        self.actions_at(id, group, None, item_id)
    }

    pub fn actions_at(
        &self,
        id: &str,
        group: &str,
        location: Option<&str>,
        item_id: &str,
    ) -> Result<Vec<Action>, String> {
        let plugin = self.active(id)?;
        let item = self
            .view(id, group, location)?
            .items
            .into_iter()
            .find(|item| item.id == item_id)
            .ok_or_else(|| format!("Item not found: {item_id}"))?;
        catch_unwind(AssertUnwindSafe(|| plugin.actions(&item)))
            .map_err(|_| format!("Plugin {id} failed while listing actions"))
    }

    pub fn commands(&self) -> Vec<&Command> {
        self.registry.values().map(|(_, command)| command).collect()
    }

    pub fn command_owner(&self, id: &str) -> Option<&str> {
        self.registry.get(id).map(|(owner, _)| owner.as_str())
    }

    pub fn invoke(&self, invocation: CommandInvocation) -> Result<CommandOutcome, String> {
        let (plugin_id, command) = self
            .registry
            .get(&invocation.id)
            .ok_or_else(|| format!("Unknown command: {}", invocation.id))?;
        if command.requires_item && invocation.item.is_none() {
            return Err(format!("Command {} requires an item", invocation.id));
        }
        let plugin = self.active(plugin_id)?;
        self.check_permissions(plugin_id)?;
        catch_unwind(AssertUnwindSafe(|| {
            plugin.execute(&self.workspace, &invocation)
        }))
        .map_err(|_| {
            format!(
                "Plugin {plugin_id} failed while executing {}",
                invocation.id
            )
        })?
    }
}
