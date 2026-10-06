use std::collections::{BTreeMap, BTreeSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

mod bindings;
mod config;
pub use bindings::BindingScope;
pub mod files;
pub mod git;
pub mod runtime;
pub mod ui;
mod viewer;
pub use config::CONFIG_FILE;
use serde_json::{json, Value};

/// Draft source-level Plugin API; no dynamic ABI or isolation is implied.
pub const PLUGIN_API_VERSION: &str = "0.7";

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct KeyBinding {
    pub keys: String,
    pub command_id: String,
    pub scope: BindingScope,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Group {
    pub id: String,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Item {
    pub id: String,
    pub title: String,
    /// Native kind chosen and interpreted by the owning plugin.
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Command {
    pub id: String,
    pub title: String,
    pub requires_item: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CommandInvocation {
    pub id: String,
    pub item: Option<String>,
    /// Arguments independent of a selected domain Item.
    pub args: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Action {
    pub label: String,
    pub command_id: String,
    pub invocation: CommandInvocation,
    pub is_default: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Block {
    pub source: String,
    pub status: String,
    pub content: String,
    #[serde(default)]
    pub format: ContentFormat,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ContentFormat {
    #[default]
    Text,
    Markdown,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ViewRequest {
    Find(Option<String>),
    GoToLine(Option<usize>),
    NextMatch { backwards: bool },
    ToggleSource,
}

/// Location and item identifiers are opaque to Core, and interpreted by a plugin.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Navigation {
    pub group: String,
    pub location: String,
    pub selected: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CommandOutcome {
    Output(Block),
    Navigate(Navigation),
    WorkspaceChanged,
    /// Core-owned navigation of the current output, independent of plugin Items.
    View(ViewRequest),
    /// Core-only signal routed to the active view's refresh lifecycle.
    Refresh,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GroupView {
    pub title: String,
    pub location: String,
    pub items: Vec<Item>,
    pub parent: Option<CommandInvocation>,
    pub command_defaults: Vec<CommandInvocation>,
}

/// Host refresh scheduling hint. Cached domain data remains owned by the plugin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RefreshStrategy {
    #[default]
    Manual,
    OnFocus,
    Interval {
        seconds: u64,
    },
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum Permission {
    WorkspaceRead,
    WorkspaceWrite,
    Process,
    Network,
    Credentials,
    Environment,
}

impl Permission {
    pub fn parse(value: &str) -> Result<Self, String> {
        serde_json::from_value(Value::String(value.into()))
            .map_err(|_| format!("Unknown permission: {value}"))
    }
    pub fn name(self) -> String {
        format!("{self:?}")
    }
}

#[derive(Clone)]
pub struct Workspace {
    root: PathBuf,
    config: Value,
    runtime_permissions: Option<BTreeSet<Permission>>,
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
            runtime_permissions: None,
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
        if self
            .runtime_permissions
            .as_ref()
            .is_some_and(|p| !p.contains(&Permission::WorkspaceRead))
        {
            return Err("WorkspaceRead not granted".into());
        }
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

    /// Resolves a writable workspace path, including a new file in an existing directory.
    pub fn write_path(&self, relative: &str) -> Result<PathBuf, String> {
        if self
            .runtime_permissions
            .as_ref()
            .is_some_and(|p| !p.contains(&Permission::WorkspaceWrite))
        {
            return Err("WorkspaceWrite not granted".into());
        }
        let path = Path::new(relative);
        if path.is_absolute()
            || path.components().any(|c| {
                !matches!(
                    c,
                    std::path::Component::Normal(_) | std::path::Component::CurDir
                )
            })
        {
            return Err("Path is outside the workspace".into());
        }
        let path = self.root.join(path);
        if path.exists() {
            let target = path.canonicalize().map_err(|e| e.to_string())?;
            if !target.starts_with(&self.root) {
                return Err("Path is outside the workspace".into());
            }
            return Ok(target);
        }
        let parent = path
            .parent()
            .ok_or("Missing parent directory")?
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if !parent.starts_with(&self.root) {
            return Err("Path is outside the workspace".into());
        }
        // Dangling symlinks must not become a write outside root.
        if std::fs::symlink_metadata(&path).is_ok() {
            return Err("Unresolved path already exists".into());
        }
        Ok(parent.join(path.file_name().ok_or("Missing filename")?))
    }
}

pub trait Plugin {
    /// Optional host polling. Linked plugins default to synchronous trusted SDK usage.
    fn poll_background(
        &self,
        workspace: &Workspace,
        permissions: &BTreeSet<Permission>,
        request: &runtime::BackgroundRequest,
    ) -> std::task::Poll<Result<runtime::BackgroundResponse, String>> {
        std::task::Poll::Ready(runtime::background_run(
            self,
            workspace,
            permissions,
            request,
        ))
    }
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn start(
        &self,
        _workspace: &Workspace,
        _permissions: &BTreeSet<Permission>,
    ) -> Result<(), String> {
        Ok(())
    }
    fn stop(&self) {}
    fn runtime_status(&self) -> runtime::RuntimeStatus {
        runtime::RuntimeStatus::in_process()
    }
    fn installation_present(&self) -> bool {
        true
    }
    fn permissions(&self) -> Vec<Permission> {
        Vec::new()
    }
    fn groups(&self) -> Vec<Group>;
    fn refresh_strategy(&self, _group: &str) -> RefreshStrategy {
        RefreshStrategy::Manual
    }
    /// Update plugin-owned state when needed and return the resulting GroupView.
    fn refresh(
        &self,
        workspace: &Workspace,
        group: &str,
        location: Option<&str>,
    ) -> Result<GroupView, String> {
        self.view(workspace, group, location)
    }
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
    fn try_actions(&self, item: &Item) -> Result<Vec<Action>, String> {
        Ok(self.actions(item))
    }
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
    pub installed: bool,
    pub workspace_enabled: bool,
    pub session_enabled: bool,
    pub runtime: runtime::RuntimeStatus,
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
    package_store: Option<runtime::PackageStore>,
    restarting: std::cell::RefCell<BTreeSet<String>>,
    discovery_errors: Vec<String>,
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
            package_store: None,
            restarting: std::cell::RefCell::new(BTreeSet::new()),
            discovery_errors: Vec::new(),
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

    pub fn load_packages(
        &mut self,
        store: runtime::PackageStore,
        enabled_defaults: &[&str],
    ) -> Vec<String> {
        self.package_store = Some(store.clone());
        let removed: Vec<_> = self
            .installed
            .iter()
            .filter(|(_, plugin)| {
                !catch_unwind(AssertUnwindSafe(|| plugin.installation_present())).unwrap_or(false)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in removed {
            self.stop_plugin(&id);
            self.remove_registration(&id);
        }
        self.discovery_errors.clear();
        match store.discover() {
            Err(error) => self.discovery_errors.push(error),
            Ok(packages) => {
                for package in packages {
                    match package {
                        Err(error) => self.discovery_errors.push(error),
                        Ok(plugin) => {
                            if self.installed.contains_key(plugin.id()) {
                                continue;
                            }
                            let enabled = enabled_defaults.contains(&plugin.id());
                            if let Err(error) = self.install(Box::new(plugin), enabled) {
                                self.discovery_errors.push(error);
                            }
                        }
                    }
                }
            }
        }
        self.discovery_errors.clone()
    }

    pub fn install_package(&mut self, source: PathBuf) -> Result<String, String> {
        let source = if source.is_absolute() {
            source
        } else {
            self.workspace.root().join(source)
        };
        let manifest = runtime::Manifest::read(&source)?;
        if self.installed.contains_key(&manifest.plugin.id) {
            return Err(format!("Plugin already installed: {}", manifest.plugin.id));
        }
        let store = self
            .package_store
            .clone()
            .ok_or("No global plugin store configured")?;
        let id = store.install(&source)?;
        let plugin = store.load(&id)?;
        if let Err(error) = self.install(Box::new(plugin), false) {
            let _ = store.uninstall(&id);
            return Err(error);
        }
        Ok(id)
    }

    pub fn uninstall(&mut self, id: &str) -> Result<(), String> {
        self.ensure_installed(id)?;
        let store = self
            .package_store
            .clone()
            .ok_or("No global plugin store configured")?;
        self.stop_plugin(id);
        store.uninstall(id)?;
        self.remove_registration(id);
        Ok(())
    }

    fn remove_registration(&mut self, id: &str) {
        self.installed.remove(id);
        self.registry
            .retain(|_, (owner, _)| owner.as_deref() != Some(id));
        self.requested_permissions.remove(id);
        self.granted_permissions.remove(id);
        self.activation_defaults.remove(id);
        self.permission_defaults.remove(id);
        self.workspace_enabled.remove(id);
        self.session_disabled.remove(id);
        self.declared_bindings
            .retain(|binding| binding.scope.plugin() != Some(id));
        // Keep project-local settings, activation intent, and saved permissions for reinstall.
    }

    pub fn trust_plugin(&mut self, id: &str) -> Result<(), String> {
        self.ensure_installed(id)?;
        self.package_store
            .as_ref()
            .ok_or("No global plugin store configured")?
            .trust(id)?;
        self.stop_plugin(id);
        Ok(())
    }

    fn stop_plugin(&self, id: &str) {
        self.restarting.borrow_mut().remove(id);
        if let Some(plugin) = self.installed.get(id) {
            let _ = catch_unwind(AssertUnwindSafe(|| plugin.stop()));
        }
    }

    pub fn restart_plugin(&self, id: &str) -> Result<(), String> {
        self.active(id)?;
        self.stop_plugin(id);
        self.prepared(id).map(|_| ())
    }

    fn prepared(&self, id: &str) -> Result<&dyn Plugin, String> {
        let plugin = self.active(id)?;
        self.check_permissions(id)?;
        let permissions = self
            .granted_permissions
            .get(id)
            .cloned()
            .unwrap_or_default();
        catch_unwind(AssertUnwindSafe(|| {
            plugin.start(&self.workspace, &permissions)
        }))
        .map_err(|_| format!("Plugin {id} panicked while starting"))??;
        Ok(plugin)
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
        let permissions = saved
            .get("permissions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_str().and_then(|name| Permission::parse(name).ok()))
            .collect();
        self.granted_permissions.insert(id, permissions);
        Ok(())
    }

    pub fn enable(&mut self, id: &str) -> Result<(), String> {
        self.ensure_installed(id)?;
        self.set_enabled(id, true)?;
        self.workspace_enabled.insert(id.to_owned());
        self.session_disabled.remove(id);
        self.stop_plugin(id);
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
            .any(|value| value.as_str() == Some(permission.name().as_str()))
        {
            permissions.push(json!(permission.name()));
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
        permissions.retain(|value| value.as_str() != Some(permission.name().as_str()));
        plugin["permissions"] = Value::Array(permissions);
        value["plugins"][id] = plugin;
        self.save_config(value)?;
        if let Some(granted) = self.granted_permissions.get_mut(id) {
            granted.remove(&permission);
        }
        self.stop_plugin(id);
        Ok(())
    }

    pub fn disable(&mut self, id: &str) -> Result<(), String> {
        self.ensure_installed(id)?;
        self.set_enabled(id, false)?;
        self.workspace_enabled.remove(id);
        self.session_disabled.remove(id);
        self.stop_plugin(id);
        Ok(())
    }
    pub fn disable_for_session(&mut self, id: &str) -> Result<(), String> {
        self.ensure_installed(id)?;
        self.session_disabled.insert(id.to_owned());
        self.stop_plugin(id);
        Ok(())
    }

    pub fn plugins(&self) -> Vec<PluginState> {
        self.installed
            .keys()
            .map(|id| PluginState {
                id: id.clone(),
                installed: catch_unwind(AssertUnwindSafe(|| {
                    self.installed[id].installation_present()
                }))
                .unwrap_or(false),
                workspace_enabled: self.workspace_enabled.contains(id),
                session_enabled: !self.session_disabled.contains(id),
                runtime: catch_unwind(AssertUnwindSafe(|| self.installed[id].runtime_status()))
                    .unwrap_or_else(|_| runtime::RuntimeStatus {
                        availability: runtime::Availability::Failed,
                        connection: runtime::Connection::Failed,
                        detail: Some("Plugin failed while reporting status".into()),
                    }),
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

    /// Resolve a registered invocation without starting a plugin or mutating state.
    pub fn validate_invocation(
        &self,
        invocation: &CommandInvocation,
    ) -> Result<Option<&str>, String> {
        let (owner, command) = self
            .registry
            .get(&invocation.id)
            .ok_or_else(|| format!("Unknown command: {}", invocation.id))?;
        if command.requires_item && invocation.item.is_none() {
            return Err(format!("Command {} requires an item", invocation.id));
        }
        if let Some(id) = owner {
            self.active(id)?;
            self.check_permissions(id)?;
        }
        Ok(owner.as_deref())
    }

    /// CommandRegistry execution with deferred delivery; Core commands remain immediate except restart.
    pub fn poll_invoke(
        &mut self,
        invocation: CommandInvocation,
    ) -> std::task::Poll<Result<CommandOutcome, String>> {
        use runtime::{BackgroundRequest as Request, BackgroundResponse as Response};
        use std::task::Poll;
        let owner = match self.validate_invocation(&invocation) {
            Ok(owner) => owner.map(str::to_owned),
            Err(error) => return Poll::Ready(Err(error)),
        };
        if let Some(owner) = owner {
            return self
                .poll_background(&owner, &Request::Execute(invocation))
                .map(|result| {
                    result.and_then(|response| match response {
                        Response::Executed(outcome) => Ok(outcome),
                        _ => Err("Unexpected command response".into()),
                    })
                });
        }
        if invocation.id == "core.plugin.restart" && invocation.args.len() == 1 {
            let id = &invocation.args[0];
            if !self.restarting.borrow().contains(id) {
                if let Err(error) = self.active(id).and_then(|_| self.check_permissions(id)) {
                    return Poll::Ready(Err(error));
                }
                self.stop_plugin(id);
                self.restarting.borrow_mut().insert(id.clone());
            }
            return self.poll_background(id, &Request::Start).map(|result| {
                self.restarting.borrow_mut().remove(id);
                result.map(|_| {
                    CommandOutcome::Output(Block {
                        format: ContentFormat::Text,
                        source: "Core".into(),
                        status: "ok".into(),
                        content: format!("{id}: connected"),
                    })
                })
            });
        }
        Poll::Ready(self.invoke(invocation))
    }

    /// Uses the same registration, activation, permissions and response validation as synchronous calls.
    pub fn poll_background(
        &self,
        id: &str,
        request: &runtime::BackgroundRequest,
    ) -> std::task::Poll<Result<runtime::BackgroundResponse, String>> {
        use runtime::{BackgroundRequest as Request, BackgroundResponse as Response};
        use std::task::Poll;
        let result = (|| {
            let plugin = self.active(id)?;
            self.check_permissions(id)?;
            if let Request::Execute(invocation) = request {
                if self.validate_invocation(invocation)? != Some(id) {
                    return Err("Command belongs to another owner".into());
                }
            }
            let permissions = self
                .granted_permissions
                .get(id)
                .cloned()
                .unwrap_or_default();
            let poll = catch_unwind(AssertUnwindSafe(|| {
                plugin.poll_background(&self.workspace, &permissions, request)
            }))
            .map_err(|_| format!("Plugin {id} panicked during background work"))?;
            match poll {
                Poll::Pending => Ok(Poll::Pending),
                Poll::Ready(result) => {
                    let response = result?;
                    match (&response, request) {
                        (Response::Started, Request::Start) => {}
                        (Response::View(view), Request::View { .. }) => {
                            self.validate_view(id, view)?
                        }
                        (Response::Refreshed(view), Request::Refresh { .. }) => {
                            self.validate_view(id, view)?
                        }
                        (Response::Actions { view, actions }, Request::Actions { .. }) => {
                            self.validate_view(id, view)?;
                            self.validate_actions(id, actions)?;
                        }
                        (Response::Executed(outcome), Request::Execute(_)) => {
                            Self::validate_outcome(outcome)?
                        }
                        _ => return Err("Background response does not match request".into()),
                    }
                    Ok(Poll::Ready(Ok(response)))
                }
            }
        })();
        result.unwrap_or_else(|error| Poll::Ready(Err(error)))
    }

    pub fn groups(&self, id: &str) -> Result<Vec<Group>, String> {
        let plugin = self.active(id)?;
        catch_unwind(AssertUnwindSafe(|| plugin.groups()))
            .map_err(|_| format!("Plugin {id} failed while listing groups"))
    }

    pub fn refresh_strategy(&self, id: &str, group: &str) -> RefreshStrategy {
        self.active(id)
            .ok()
            .and_then(|plugin| {
                catch_unwind(AssertUnwindSafe(|| plugin.refresh_strategy(group))).ok()
            })
            .unwrap_or_default()
    }

    pub fn items(&self, id: &str, group: &str) -> Result<Vec<Item>, String> {
        Ok(self.view(id, group, None)?.items)
    }

    pub fn view(&self, id: &str, group: &str, location: Option<&str>) -> Result<GroupView, String> {
        let plugin = self.prepared(id)?;
        let view = catch_unwind(AssertUnwindSafe(|| {
            plugin.view(&self.workspace, group, location)
        }))
        .map_err(|_| format!("Plugin {id} failed while loading items"))??;
        self.validate_view(id, &view)?;
        Ok(view)
    }

    fn validate_view(&self, id: &str, view: &GroupView) -> Result<(), String> {
        let mut ids = BTreeSet::new();
        if view.items.iter().any(|item| !ids.insert(&item.id)) {
            return Err("Duplicate Item id in plugin view".into());
        }
        for invocation in view.parent.iter().chain(view.command_defaults.iter()) {
            if self.command_owner(&invocation.id) != Some(id) {
                return Err("Plugin view must use its own registered commands".into());
            }
        }
        Ok(())
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
        let actions = catch_unwind(AssertUnwindSafe(|| plugin.try_actions(&item)))
            .map_err(|_| format!("Plugin {id} failed while listing actions"))??;
        self.validate_actions(id, &actions)?;
        Ok(actions)
    }

    fn validate_actions(&self, id: &str, actions: &[Action]) -> Result<(), String> {
        if actions.iter().filter(|action| action.is_default).count() > 1 {
            return Err("Multiple default actions".into());
        }
        for action in actions {
            if action.command_id != action.invocation.id
                || self.command_owner(&action.command_id) != Some(id)
            {
                return Err("Plugin action must invoke its own registered command".into());
            }
        }
        Ok(())
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
            let permissions = saved
                .get("permissions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|value| value.as_str().and_then(|name| Permission::parse(name).ok()))
                .collect();
            granted.insert(id.clone(), permissions);
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
        for id in self.installed.keys() {
            self.stop_plugin(id);
        }
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

    pub fn invoke(&mut self, mut invocation: CommandInvocation) -> Result<CommandOutcome, String> {
        // Legacy read aliases resolve to the same canonical operation and registry route.
        if matches!(
            invocation.id.as_str(),
            "core.permission.grant-read" | "core.permission.revoke-read"
        ) {
            if invocation.args.len() != 1 {
                return Err(format!("{} requires one plugin id", invocation.id));
            }
            invocation.id = if invocation.id.ends_with("grant-read") {
                "core.permission.grant"
            } else {
                "core.permission.revoke"
            }
            .into();
            invocation.args.push("WorkspaceRead".into());
        }
        let Some(plugin_id) = self.validate_invocation(&invocation)? else {
            return self.invoke_core(invocation);
        };
        let plugin = self.prepared(plugin_id)?;
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            plugin.execute(&self.workspace, &invocation)
        }))
        .map_err(|_| {
            format!(
                "Plugin {plugin_id} failed while executing {}",
                invocation.id
            )
        })??;
        Self::validate_outcome(&outcome)?;
        Ok(outcome)
    }

    fn validate_outcome(outcome: &CommandOutcome) -> Result<(), String> {
        if matches!(
            outcome,
            CommandOutcome::WorkspaceChanged | CommandOutcome::View(_) | CommandOutcome::Refresh
        ) {
            return Err("Core-only command outcome returned by plugin".into());
        }
        Ok(())
    }

    fn invoke_core(&mut self, invocation: CommandInvocation) -> Result<CommandOutcome, String> {
        if invocation.id.starts_with("core.view.") {
            let arg = match invocation.args.as_slice() {
                [] => None,
                [arg] => Some(arg),
                _ => return Err("View command accepts at most one argument".into()),
            };
            let request = match invocation.id.as_str() {
                "core.view.find" => ViewRequest::Find(arg.cloned()),
                "core.view.goto" => ViewRequest::GoToLine(
                    arg.map(|arg| {
                        arg.parse::<usize>()
                            .ok()
                            .filter(|line| *line > 0)
                            .ok_or("Line number must be a positive integer")
                    })
                    .transpose()?,
                ),
                "core.view.next" | "core.view.previous" | "core.view.source" if arg.is_some() => {
                    return Err("This view command takes no arguments".into())
                }
                "core.view.next" => ViewRequest::NextMatch { backwards: false },
                "core.view.previous" => ViewRequest::NextMatch { backwards: true },
                "core.view.source" => ViewRequest::ToggleSource,
                _ => return Err("Unknown view command".into()),
            };
            return Ok(CommandOutcome::View(request));
        }
        if invocation.id == "core.refresh" {
            if !invocation.args.is_empty() {
                return Err("core.refresh takes no arguments".into());
            }
            return Ok(CommandOutcome::Refresh);
        }
        let content = match invocation.id.as_str() {
            "core.plugin.install" => {
                if invocation.args.len() != 1 {
                    return Err("core.plugin.install requires one package directory".into());
                }
                let id = self.install_package(PathBuf::from(&invocation.args[0]))?;
                format!("{id}: installed; explicit trust required; activation follows saved Workspace configuration")
            }
            "core.plugins.discover" => {
                if !invocation.args.is_empty() {
                    return Err("core.plugins.discover takes no arguments".into());
                }
                let store = self
                    .package_store
                    .clone()
                    .ok_or("No global plugin store configured")?;
                let errors = self.load_packages(store, &[]);
                if errors.is_empty() {
                    "Discovery complete; no new Workspace activation defaults".into()
                } else {
                    errors.join("\n")
                }
            }
            "core.permission.grant" | "core.permission.revoke" => {
                if invocation.args.len() != 2 {
                    return Err(format!(
                        "{} requires plugin id and permission name",
                        invocation.id
                    ));
                }
                let id = &invocation.args[0];
                self.ensure_installed(id)?;
                let permission = Permission::parse(&invocation.args[1])?;
                if invocation.id == "core.permission.grant" {
                    self.grant(id, permission)?;
                    format!("{id}: {permission:?} granted")
                } else {
                    self.revoke(id, permission)?;
                    format!("{id}: {permission:?} revoked")
                }
            }
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
                    std::iter::once("No installed plugins".to_owned())
                        .chain(
                            self.discovery_errors
                                .iter()
                                .map(|error| format!("Discovery error: {error}")),
                        )
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    let mut lines = states
                        .into_iter()
                        .map(|plugin| {
                            format!(
                                "{}: {}\nInstalled: {}\nWorkspace enabled: {}\nSession enabled: {}\nAvailability: {:?}\nConnection: {:?}{}\n",
                                plugin.id,
                                plugin.status,
                                plugin.installed,
                                plugin.workspace_enabled,
                                plugin.session_enabled,
                                plugin.runtime.availability,
                                plugin.runtime.connection,
                                plugin
                                    .runtime
                                    .detail
                                    .map(|detail| format!("\n{detail}"))
                                    .unwrap_or_default()
                            )
                        })
                        .collect::<Vec<_>>();
                    lines.extend(
                        self.discovery_errors
                            .iter()
                            .map(|error| format!("Discovery error: {error}")),
                    );
                    lines.join("\n")
                }
            }
            id => {
                if invocation.args.len() != 1 {
                    return Err(format!("{id} requires one plugin id"));
                }
                let plugin = &invocation.args[0];
                self.ensure_installed(plugin)?;
                match id {
                    "core.plugin.untrust" => {
                        self.package_store
                            .as_ref()
                            .ok_or("No global plugin store configured")?
                            .untrust(plugin)?;
                        self.stop_plugin(plugin);
                        format!("{plugin}: trust revoked; process stopped")
                    }
                    "core.plugin.trust" => {
                        self.trust_plugin(plugin)?;
                        format!("{plugin}: trusted for local native execution; this runtime is not an OS sandbox")
                    }
                    "core.plugin.uninstall" => {
                        self.uninstall(plugin)?;
                        format!("{plugin}: uninstalled; Workspace settings preserved")
                    }
                    "core.plugin.restart" => {
                        self.restart_plugin(plugin)?;
                        format!("{plugin}: connected")
                    }
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
                    "core.permissions" => {
                        let requested = &self.requested_permissions[plugin];
                        let granted = self
                            .granted_permissions
                            .get(plugin)
                            .cloned()
                            .unwrap_or_default();
                        let mut lines = vec![plugin.clone()];
                        for permission in requested.union(&granted) {
                            lines.push(format!(
                                "{permission:?}: requested={} granted={}",
                                requested.contains(permission),
                                granted.contains(permission)
                            ));
                        }
                        if requested.is_empty() && granted.is_empty() {
                            lines.push("No declared permissions".into());
                        }
                        lines.push("Host checks; no OS sandbox".into());
                        lines.join("\n")
                    }
                    _ => return Err(format!("Unknown core command: {id}")),
                }
            }
        };
        Ok(CommandOutcome::Output(Block {
            format: ContentFormat::Text,
            source: "Core".into(),
            status: "ok".into(),
            content,
        }))
    }
}

fn core_commands() -> Vec<Command> {
    [
        ("core.refresh", "Refresh current plugin group"),
        ("core.view.find", "Find text in viewed output"),
        ("core.view.goto", "Go to source line in viewed output"),
        ("core.view.next", "Next matching line in viewed output"),
        (
            "core.view.previous",
            "Previous matching line in viewed output",
        ),
        (
            "core.view.source",
            "Toggle Markdown/source in viewed output",
        ),
        (
            "core.plugin.install",
            "Install local plugin package (directory)",
        ),
        (
            "core.plugins.discover",
            "Discover globally installed plugins",
        ),
        (
            "core.plugin.uninstall",
            "Uninstall selected plugin globally",
        ),
        (
            "core.plugin.trust",
            "Trust selected plugin for native execution",
        ),
        (
            "core.plugin.untrust",
            "Revoke trust and stop selected plugin",
        ),
        ("core.plugin.restart", "Restart selected plugin"),
        (
            "core.permission.grant",
            "Grant a declared permission (plugin name)",
        ),
        (
            "core.permission.revoke",
            "Revoke a permission (plugin name)",
        ),
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

impl Drop for App {
    fn drop(&mut self) {
        for id in self.installed.keys() {
            self.stop_plugin(id);
        }
    }
}
