use std::collections::{BTreeMap, BTreeSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

mod bindings;
mod config;
pub use bindings::BindingScope;
pub mod files;
pub mod ui;
pub use config::CONFIG_FILE;
use serde_json::{json, Value};

/// Draft source-level Plugin API; no dynamic ABI or isolation is implied.
pub const PLUGIN_API_VERSION: &str = "0.3";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyBinding {
    pub keys: String,
    pub command_id: String,
    pub scope: BindingScope,
}

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
    /// Arguments independent of a selected domain Item.
    pub args: Vec<String>,
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
    WorkspaceChanged,
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
    config: Value,
}

impl Workspace {
    pub fn open(root: PathBuf) -> Result<Self, String> {
        let root = root.canonicalize().map_err(|error| error.to_string())?;
        if !root.is_dir() {
            return Err(format!("Not a directory: {}", root.display()));
        }
        Ok(Self {
            root,
            config: config::defaults(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn plugin_settings(&self, id: &str) -> Option<&Value> {
        self.config.get("plugins")?.get(id)?.get("settings")
    }

    pub fn overrides(&self) -> Option<&Value> {
        self.config.get("overrides")
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
    /// A single terminal cell identifying the native item kind; Core uses a dot fallback.
    fn item_icon(&self, _item: &Item) -> char {
        '•'
    }
    fn actions(&self, item: &Item) -> Vec<Action>;
    fn commands(&self) -> Vec<Command>;
    fn keybindings(&self) -> Vec<KeyBinding> {
        Vec::new()
    }
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
    // None owns a Core command; plugins cannot register the reserved core namespace.
    registry: BTreeMap<String, (Option<String>, Command)>,
    requested_permissions: BTreeMap<String, BTreeSet<Permission>>,
    granted_permissions: BTreeMap<String, BTreeSet<Permission>>,
    store: Option<config::Store>,
    activation_defaults: BTreeMap<String, bool>,
    permission_defaults: BTreeMap<String, BTreeSet<Permission>>,
    declared_bindings: Vec<KeyBinding>,
    sessions: BTreeMap<PathBuf, (BTreeSet<String>, Value)>,
    previous_workspace: Option<PathBuf>,
}

impl App {
    pub fn new(root: PathBuf) -> Result<Self, String> {
        Ok(Self {
            workspace: Workspace::open(root)?,
            installed: BTreeMap::new(),
            workspace_enabled: BTreeSet::new(),
            session_disabled: BTreeSet::new(),
            registry: core_commands()
                .into_iter()
                .map(|command| (command.id.clone(), (None, command)))
                .collect(),
            requested_permissions: BTreeMap::new(),
            granted_permissions: BTreeMap::new(),
            store: None,
            activation_defaults: BTreeMap::new(),
            permission_defaults: BTreeMap::new(),
            declared_bindings: bindings::core_bindings(),
            sessions: BTreeMap::new(),
            previous_workspace: None,
        })
    }

    /// Opens project-local persistent configuration. Read failures leave a usable,
    /// fail-closed session; configuration writes remain blocked until restart.
    pub fn open(root: PathBuf) -> Result<Self, String> {
        let mut app = Self::new(root)?;
        let (store, value) = config::Store::load(app.workspace.root());
        app.workspace.config = value;
        app.store = Some(store);
        Ok(app)
    }

    pub fn configuration_error(&self) -> Option<&str> {
        self.store.as_ref().and_then(|store| store.error.as_deref())
    }

    fn save_config(&mut self, value: Value) -> Result<(), String> {
        if let Some(store) = &mut self.store {
            store.save(&value)?;
        }
        self.workspace.config = value;
        Ok(())
    }

    fn ensure_installed(&self, id: &str) -> Result<(), String> {
        if self.installed.contains_key(id) {
            Ok(())
        } else {
            Err(format!("Plugin not installed: {id}"))
        }
    }

    fn plugin_config(&self, id: &str) -> Value {
        self.workspace.config["plugins"]
            .get(id)
            .cloned()
            .unwrap_or_else(|| json!({}))
    }

    /// An explicit application default, applied only without a saved permission decision.
    pub fn grant_default(&mut self, id: &str, permission: Permission) -> Result<(), String> {
        self.ensure_installed(id)?;
        if !self.requested_permissions[id].contains(&permission) {
            return Err(format!("Plugin {id} did not request {permission:?}"));
        }
        self.permission_defaults
            .entry(id.into())
            .or_default()
            .insert(permission);
        if self.configuration_error().is_none()
            && self.plugin_config(id).get("permissions").is_none()
        {
            self.grant(id, permission)?;
        }
        Ok(())
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    pub fn install(&mut self, plugin: Box<dyn Plugin>, enabled: bool) -> Result<(), String> {
        let id = plugin.id().to_owned();
        if id == "core" || id.is_empty() {
            return Err(format!("Reserved or empty plugin id: {id}"));
        }
        if self.installed.contains_key(&id) {
            return Err(format!("Plugin already installed: {id}"));
        }
        let commands = catch_unwind(AssertUnwindSafe(|| plugin.commands()))
            .map_err(|_| format!("Plugin {id} panicked while registering commands"))?;
        let permissions = catch_unwind(AssertUnwindSafe(|| plugin.permissions()))
            .map_err(|_| format!("Plugin {id} panicked while declaring permissions"))?;
        let keybindings = catch_unwind(AssertUnwindSafe(|| plugin.keybindings()))
            .map_err(|_| format!("Plugin {id} panicked while declaring bindings"))?;
        let mut new_ids = BTreeSet::new();
        for command in &commands {
            if !new_ids.insert(command.id.clone())
                || self.registry.contains_key(&command.id)
                || !command.id.starts_with(&format!("{id}."))
            {
                return Err(format!("Invalid or duplicate command: {}", command.id));
            }
        }
        for binding in &keybindings {
            bindings::validate_keys(&binding.keys, &binding.scope)?;
            if binding.scope.plugin() != Some(id.as_str()) || !new_ids.contains(&binding.command_id)
            {
                return Err(format!("Invalid binding for plugin {id}: {}", binding.keys));
            }
        }
        for command in commands {
            self.registry
                .insert(command.id.clone(), (Some(id.clone()), command));
        }
        self.installed.insert(id.clone(), plugin);
        self.activation_defaults.insert(id.clone(), enabled);
        self.declared_bindings.extend(keybindings);
        self.requested_permissions
            .insert(id.clone(), permissions.into_iter().collect());
        let saved = self.plugin_config(&id);
        if self.configuration_error().is_none()
            && saved
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(enabled)
        {
            self.workspace_enabled.insert(id.clone());
        }
        if saved["permissions"].as_array().is_some_and(|values| {
            values
                .iter()
                .any(|value| value.as_str() == Some("WorkspaceRead"))
        }) {
            self.granted_permissions
                .entry(id)
                .or_default()
                .insert(Permission::WorkspaceRead);
        }
        Ok(())
    }

    pub fn enable(&mut self, id: &str) -> Result<(), String> {
        self.ensure_installed(id)?;
        self.set_enabled(id, true)?;
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
        let mut value = self.workspace.config.clone();
        let mut plugin = self.plugin_config(id);
        plugin["enabled"] = json!(self.workspace_enabled.contains(id));
        let mut permissions = plugin
            .get("permissions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if !permissions
            .iter()
            .any(|value| value.as_str() == Some("WorkspaceRead"))
        {
            permissions.push(json!("WorkspaceRead"));
        }
        plugin["permissions"] = Value::Array(permissions);
        value["plugins"][id] = plugin;
        self.save_config(value)?;
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

    fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<(), String> {
        let mut value = self.workspace.config.clone();
        let mut plugin = self.plugin_config(id);
        plugin["enabled"] = json!(enabled);
        value["plugins"][id] = plugin;
        self.save_config(value)
    }

    pub fn revoke(&mut self, id: &str, permission: Permission) -> Result<(), String> {
        self.ensure_installed(id)?;
        let mut value = self.workspace.config.clone();
        let mut plugin = self.plugin_config(id);
        plugin["enabled"] = json!(self.workspace_enabled.contains(id));
        let mut permissions = plugin
            .get("permissions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        permissions.retain(|value| value.as_str() != Some("WorkspaceRead"));
        plugin["permissions"] = Value::Array(permissions);
        value["plugins"][id] = plugin;
        self.save_config(value)?;
        if let Some(granted) = self.granted_permissions.get_mut(id) {
            granted.remove(&permission);
        }
        Ok(())
    }

    pub fn disable(&mut self, id: &str) -> Result<(), String> {
        self.ensure_installed(id)?;
        self.set_enabled(id, false)?;
        self.workspace_enabled.remove(id);
        self.session_disabled.remove(id);
        Ok(())
    }
    pub fn disable_for_session(&mut self, id: &str) -> Result<(), String> {
        self.ensure_installed(id)?;
        self.session_disabled.insert(id.to_owned());
        Ok(())
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

    pub fn item_icon(&self, id: &str, item: &Item) -> char {
        self.active(id)
            .ok()
            .and_then(|plugin| catch_unwind(AssertUnwindSafe(|| plugin.item_icon(item))).ok())
            .filter(|icon| {
                !icon.is_control() && console::measure_text_width(&icon.to_string()) == 1
            })
            .unwrap_or('•')
    }

    pub fn commands(&self) -> Vec<&Command> {
        self.registry.values().map(|(_, command)| command).collect()
    }

    fn binding_table(&self) -> bindings::BindingTable {
        bindings::BindingTable::new(
            self.declared_bindings.clone(),
            self.workspace.overrides(),
            &self.registry.keys().cloned().collect(),
        )
    }

    pub fn keybindings(
        &self,
        plugin: Option<&str>,
        group: Option<&str>,
        location: Option<&str>,
    ) -> Vec<KeyBinding> {
        let plugin = plugin.filter(|id| self.active(id).is_ok());
        self.binding_table().resolve(plugin, group, location).0
    }

    pub fn binding_diagnostics(&self) -> Vec<String> {
        self.binding_table().diagnostics()
    }

    /// Prepare configuration and defaults before replacing the current context.
    /// Installed implementations and the command registry remain unchanged.
    pub fn switch_workspace(&mut self, root: PathBuf) -> Result<(), String> {
        let mut workspace = Workspace::open(if root.is_absolute() {
            root
        } else {
            self.workspace.root().join(root)
        })?;
        if workspace.root() == self.workspace.root() {
            return Ok(());
        }
        let mut store = if self.store.is_some() {
            let (store, value) = config::Store::load(workspace.root());
            if let Some(error) = &store.error {
                return Err(error.clone());
            }
            workspace.config = value;
            Some(store)
        } else {
            if let Some((_, value)) = self.sessions.get(workspace.root()) {
                workspace.config = value.clone();
            }
            None
        };
        let mut enabled = BTreeSet::new();
        let mut granted = BTreeMap::new();
        let original = workspace.config.clone();
        for (id, default) in &self.activation_defaults {
            let mut saved = workspace.config["plugins"]
                .get(id)
                .cloned()
                .unwrap_or_else(|| json!({}));
            let active = saved
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(*default);
            if active {
                enabled.insert(id.clone());
            }
            if saved.get("permissions").is_none() {
                if let Some(defaults) = self.permission_defaults.get(id) {
                    saved["enabled"] = json!(active);
                    saved["permissions"] = json!(defaults
                        .iter()
                        .map(|p| format!("{p:?}"))
                        .collect::<Vec<_>>());
                    workspace.config["plugins"][id] = saved.clone();
                }
            }
            if saved["permissions"].as_array().is_some_and(|permissions| {
                permissions
                    .iter()
                    .any(|p| p.as_str() == Some("WorkspaceRead"))
            }) {
                granted.insert(id.clone(), BTreeSet::from([Permission::WorkspaceRead]));
            }
        }
        if original != workspace.config {
            if let Some(store) = &mut store {
                store.save(&workspace.config)?;
            }
        }
        let session_disabled = self
            .sessions
            .get(workspace.root())
            .map(|(disabled, _)| disabled.clone())
            .unwrap_or_default();
        let old_root = self.workspace.root().to_owned();
        self.sessions.insert(
            old_root.clone(),
            (self.session_disabled.clone(), self.workspace.config.clone()),
        );
        self.workspace = workspace;
        self.store = store;
        self.workspace_enabled = enabled;
        self.granted_permissions = granted;
        self.session_disabled = session_disabled;
        self.previous_workspace = Some(old_root);
        Ok(())
    }

    pub fn command_owner(&self, id: &str) -> Option<&str> {
        self.registry
            .get(id)
            .and_then(|(owner, _)| owner.as_deref())
    }

    pub fn invoke(&mut self, invocation: CommandInvocation) -> Result<CommandOutcome, String> {
        let (plugin_id, command) = self
            .registry
            .get(&invocation.id)
            .ok_or_else(|| format!("Unknown command: {}", invocation.id))?;
        if command.requires_item && invocation.item.is_none() {
            return Err(format!("Command {} requires an item", invocation.id));
        }
        let Some(plugin_id) = plugin_id else {
            return self.invoke_core(invocation);
        };
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

    fn invoke_core(&mut self, invocation: CommandInvocation) -> Result<CommandOutcome, String> {
        let content = match invocation.id.as_str() {
            "core.workspace.open" => {
                if invocation.args.len() != 1 {
                    return Err("core.workspace.open requires one project path".into());
                }
                self.switch_workspace(PathBuf::from(&invocation.args[0]))?;
                return Ok(CommandOutcome::WorkspaceChanged);
            }
            "core.workspace.previous" => {
                if !invocation.args.is_empty() {
                    return Err("core.workspace.previous takes no arguments".into());
                }
                let root = self
                    .previous_workspace
                    .clone()
                    .ok_or("No previous Workspace")?;
                self.switch_workspace(root)?;
                return Ok(CommandOutcome::WorkspaceChanged);
            }
            "core.plugins" => {
                if !invocation.args.is_empty() {
                    return Err("core.plugins takes no arguments".into());
                }
                let states = self.plugins();
                if states.is_empty() {
                    "No installed plugins".into()
                } else {
                    states
                        .into_iter()
                        .map(|plugin| format!("{}: {}", plugin.id, plugin.status))
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            }
            id => {
                if invocation.args.len() != 1 {
                    return Err(format!("{id} requires one plugin id"));
                }
                let plugin = &invocation.args[0];
                self.ensure_installed(plugin)?;
                match id {
                    "core.plugin.enable" => {
                        self.enable(plugin)?;
                        format!("{plugin}: enabled for Workspace and session")
                    }
                    "core.plugin.disable" => {
                        self.disable(plugin)?;
                        format!("{plugin}: workspace disabled")
                    }
                    "core.plugin.suspend" => {
                        self.disable_for_session(plugin)?;
                        format!("{plugin}: session disabled")
                    }
                    "core.permission.grant-read" => {
                        self.grant(plugin, Permission::WorkspaceRead)?;
                        format!("{plugin}: WorkspaceRead granted")
                    }
                    "core.permission.revoke-read" => {
                        self.revoke(plugin, Permission::WorkspaceRead)?;
                        format!("{plugin}: WorkspaceRead revoked")
                    }
                    "core.permissions" => format!(
                        "{plugin}\nRequested: {:?}\nGranted: {:?}",
                        self.requested_permissions.get(plugin).unwrap(),
                        self.granted_permissions
                            .get(plugin)
                            .cloned()
                            .unwrap_or_default()
                    ),
                    _ => return Err(format!("Unknown core command: {id}")),
                }
            }
        };
        Ok(CommandOutcome::Output(Block {
            source: "Core".into(),
            status: "ok".into(),
            content,
        }))
    }
}

fn core_commands() -> Vec<Command> {
    [
        ("core.workspace.open", "Open Workspace (project path)"),
        ("core.workspace.previous", "Switch to previous Workspace"),
        ("core.plugins", "List installed plugins and activation"),
        (
            "core.plugin.enable",
            "Enable plugin for Workspace and session",
        ),
        ("core.plugin.disable", "Disable plugin for Workspace"),
        ("core.plugin.suspend", "Disable plugin until session ends"),
        ("core.permissions", "Show plugin permissions"),
        (
            "core.permission.grant-read",
            "Grant WorkspaceRead permission",
        ),
        (
            "core.permission.revoke-read",
            "Revoke WorkspaceRead permission",
        ),
    ]
    .into_iter()
    .map(|(id, title)| Command {
        id: id.into(),
        title: title.into(),
        requires_item: false,
    })
    .collect()
}
