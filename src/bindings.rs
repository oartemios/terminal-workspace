use crate::KeyBinding;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Scope identifiers are native plugin identifiers, including opaque view locations.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BindingScope {
    Global,
    Plugin(String),
    Group {
        plugin: String,
        group: String,
    },
    View {
        plugin: String,
        group: String,
        location: String,
    },
}

impl BindingScope {
    pub(crate) fn plugin(&self) -> Option<&str> {
        match self {
            Self::Global => None,
            Self::Plugin(plugin) | Self::Group { plugin, .. } | Self::View { plugin, .. } => {
                Some(plugin)
            }
        }
    }

    fn rank(
        &self,
        plugin: Option<&str>,
        group: Option<&str>,
        location: Option<&str>,
    ) -> Option<u8> {
        match self {
            Self::Global => Some(0),
            Self::Plugin(id) if Some(id.as_str()) == plugin => Some(1),
            Self::Group {
                plugin: id,
                group: name,
            } if Some(id.as_str()) == plugin && Some(name.as_str()) == group => Some(2),
            Self::View {
                plugin: id,
                group: name,
                location: path,
            } if Some(id.as_str()) == plugin
                && Some(name.as_str()) == group
                && Some(path.as_str()) == location =>
            {
                Some(3)
            }
            _ => None,
        }
    }
}

pub(crate) fn validate_keys(keys: &str, scope: &BindingScope) -> Result<(), String> {
    let first = keys.chars().next().ok_or("Empty binding")?;
    if keys
        .chars()
        .any(|key| key.is_control() || key.is_whitespace())
    {
        return Err(format!(
            "Binding {keys:?}: only printable non-space characters are supported"
        ));
    }
    if "qhjklrsa:/?".contains(first) || (first == ',' && !matches!(scope, BindingScope::Global)) {
        return Err(format!(
            "Binding {keys:?}: first key is reserved for UI/Core"
        ));
    }
    Ok(())
}

pub(crate) fn core_bindings() -> Vec<KeyBinding> {
    [
        (",e", "core.plugin.enable"),
        (",d", "core.plugin.disable"),
        (",s", "core.plugin.suspend"),
        (",p", "core.plugins"),
        (",g", "core.permission.grant-read"),
        (",r", "core.permission.revoke-read"),
        (",w", "core.workspace.open"),
        (",b", "core.workspace.previous"),
    ]
    .into_iter()
    .map(|(keys, command_id)| KeyBinding {
        keys: keys.into(),
        command_id: command_id.into(),
        scope: BindingScope::Global,
    })
    .collect()
}

pub(crate) struct BindingTable {
    entries: Vec<KeyBinding>,
    override_keys: BTreeSet<(BindingScope, String)>,
    disabled: BTreeSet<(BindingScope, String)>,
    pub errors: Vec<String>,
}

impl BindingTable {
    pub fn new(
        base: Vec<KeyBinding>,
        overrides: Option<&Value>,
        commands: &BTreeSet<String>,
    ) -> Self {
        let mut errors = Vec::new();
        let mut override_keys = BTreeSet::new();
        let mut disabled = BTreeSet::new();
        let mut entries: BTreeMap<(BindingScope, String), Vec<KeyBinding>> = BTreeMap::new();
        for binding in base {
            entries
                .entry((binding.scope.clone(), binding.keys.clone()))
                .or_default()
                .push(binding);
        }
        if let Some(value) = overrides.and_then(|value| value.get("keybindings")) {
            if let Some(values) = value.as_array() {
                for value in values {
                    let parsed = (|| {
                        let keys = value
                            .get("keys")
                            .and_then(Value::as_str)
                            .ok_or("Expected binding keys string")?
                            .to_owned();
                        let plugin = value
                            .get("plugin")
                            .map(|v| v.as_str().ok_or("Expected plugin string"))
                            .transpose()?;
                        let group = value
                            .get("group")
                            .map(|v| v.as_str().ok_or("Expected group string"))
                            .transpose()?;
                        let location = value
                            .get("location")
                            .map(|v| v.as_str().ok_or("Expected location string"))
                            .transpose()?;
                        let scope = match (plugin, group, location) {
                            (None, None, None) => BindingScope::Global,
                            (Some(id), None, None) => BindingScope::Plugin(id.into()),
                            (Some(id), Some(group), None) => BindingScope::Group {
                                plugin: id.into(),
                                group: group.into(),
                            },
                            (Some(id), Some(group), Some(location)) => BindingScope::View {
                                plugin: id.into(),
                                group: group.into(),
                                location: location.into(),
                            },
                            _ => {
                                return Err(
                                    "group requires plugin; location requires plugin and group"
                                        .to_owned(),
                                )
                            }
                        };
                        validate_keys(&keys, &scope)?;
                        let command = value
                            .get("command")
                            .ok_or("Expected command string or null")?;
                        let command_id = if command.is_null() {
                            None
                        } else {
                            Some(
                                command
                                    .as_str()
                                    .ok_or("Expected command string or null")?
                                    .to_owned(),
                            )
                        };
                        if let Some(id) = &command_id {
                            if !commands.contains(id) {
                                return Err(format!("Unknown binding command: {id}"));
                            }
                            if let Some(plugin) = scope.plugin() {
                                if !id.starts_with(&format!("{plugin}.")) {
                                    return Err(format!(
                                        "Binding {keys}: command must belong to {plugin}"
                                    ));
                                }
                            } else if !id.starts_with("core.") {
                                return Err("Global overrides must use Core commands".into());
                            }
                        }
                        Ok((scope, keys, command_id))
                    })();
                    match parsed {
                        Ok((scope, keys, command_id)) => {
                            let key = (scope.clone(), keys.clone());
                            if !override_keys.insert(key.clone()) {
                                errors.push(format!("Duplicate binding override: {keys}"));
                                entries.remove(&key);
                                disabled.insert(key);
                                continue;
                            }
                            entries.remove(&key);
                            if let Some(command_id) = command_id {
                                entries.insert(
                                    key,
                                    vec![KeyBinding {
                                        keys,
                                        command_id,
                                        scope,
                                    }],
                                );
                            } else {
                                disabled.insert(key);
                            }
                        }
                        Err(error) => errors.push(error),
                    }
                }
            } else {
                errors.push("Expected overrides.keybindings array".into());
            }
        }
        let mut flattened = Vec::new();
        for ((_, keys), candidates) in entries {
            if candidates.len() != 1 {
                errors.push(format!("Conflicting binding: {keys}"));
            } else {
                flattened.extend(candidates);
            }
        }
        Self {
            entries: flattened,
            override_keys,
            disabled,
            errors,
        }
    }

    pub fn resolve(
        &self,
        plugin: Option<&str>,
        group: Option<&str>,
        location: Option<&str>,
    ) -> (Vec<KeyBinding>, Vec<String>) {
        let mut by_keys: BTreeMap<String, (u8, Option<KeyBinding>)> = BTreeMap::new();
        for binding in &self.entries {
            if let Some(rank) = binding.scope.rank(plugin, group, location) {
                let rank = rank
                    + if self
                        .override_keys
                        .contains(&(binding.scope.clone(), binding.keys.clone()))
                    {
                        4
                    } else {
                        0
                    };
                if by_keys
                    .get(&binding.keys)
                    .map_or(true, |(old, _)| rank > *old)
                {
                    by_keys.insert(binding.keys.clone(), (rank, Some(binding.clone())));
                }
            }
        }
        for (scope, keys) in &self.disabled {
            if let Some(rank) = scope.rank(plugin, group, location) {
                let rank = rank + 4;
                if by_keys.get(keys).map_or(true, |(old, _)| rank >= *old) {
                    by_keys.insert(keys.clone(), (rank, None));
                }
            }
        }
        by_keys.retain(|_, (_, binding)| binding.is_some());
        let mut conflicts = BTreeSet::new();
        let mut errors = Vec::new();
        for a in by_keys.keys() {
            for b in by_keys.keys() {
                if a != b && b.starts_with(a) {
                    conflicts.insert(a.clone());
                    conflicts.insert(b.clone());
                    errors.push(format!("Ambiguous binding prefix: {a} / {b}"));
                }
            }
        }
        (
            by_keys
                .into_values()
                .filter_map(|(_, binding)| binding)
                .filter(|binding| !conflicts.contains(&binding.keys))
                .collect(),
            errors,
        )
    }

    pub fn diagnostics(&self) -> Vec<String> {
        let mut errors = self.errors.clone();
        errors.extend(self.resolve(None, None, None).1);
        for binding in &self.entries {
            let (plugin, group, location) = match &binding.scope {
                BindingScope::Global => (None, None, None),
                BindingScope::Plugin(id) => (Some(id.as_str()), None, None),
                BindingScope::Group { plugin, group } => {
                    (Some(plugin.as_str()), Some(group.as_str()), None)
                }
                BindingScope::View {
                    plugin,
                    group,
                    location,
                } => (
                    Some(plugin.as_str()),
                    Some(group.as_str()),
                    Some(location.as_str()),
                ),
            };
            errors.extend(self.resolve(plugin, group, location).1);
        }
        errors.sort();
        errors.dedup();
        errors
    }
}
