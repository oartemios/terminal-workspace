use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

pub const CONFIG_FILE: &str = ".terminal-workspace.json";
static NEXT_SAVE: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn defaults() -> Value {
    json!({"version": 1, "plugins": {}, "overrides": {}})
}

pub(crate) struct Store {
    path: PathBuf,
    original: Option<Vec<u8>>,
    pub error: Option<String>,
}

impl Store {
    pub fn load(root: &Path) -> (Self, Value) {
        let path = root.join(CONFIG_FILE);
        let mut store = Self {
            path,
            original: None,
            error: None,
        };
        let result = store.read().and_then(|bytes| {
            let value = match &bytes {
                Some(bytes) => serde_json::from_slice(bytes).map_err(|error| error.to_string())?,
                None => defaults(),
            };
            validate(&value)?;
            store.original = bytes;
            Ok(value)
        });
        match result {
            Ok(value) => (store, value),
            Err(error) => {
                store.error = Some(format!(
                    "Configuration: {error}; writes disabled until restart ({})",
                    store.path.display()
                ));
                (store, defaults())
            }
        }
    }

    fn read(&self) -> Result<Option<Vec<u8>>, String> {
        match fs::symlink_metadata(&self.path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
            Ok(metadata) if !metadata.is_file() => {
                return Err("Expected a regular file (symlinks are unsupported)".into())
            }
            Ok(_) => {}
        }
        let mut bytes = Vec::new();
        fs::File::open(&self.path)
            .map_err(|error| error.to_string())?
            .take(1_048_577)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() > 1_048_576 {
            return Err("Configuration exceeds 1 MiB".into());
        }
        Ok(Some(bytes))
    }

    pub fn save(&mut self, value: &Value) -> Result<(), String> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        if self.read()? != self.original {
            return Err("Configuration changed on disk; restart before saving".into());
        }
        let mut bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
        bytes.push(b'\n');
        if bytes.len() > 1_048_576 {
            return Err("Configuration exceeds 1 MiB".into());
        }
        let temp = self.path.with_file_name(format!(
            "{CONFIG_FILE}.tmp-{}-{}",
            std::process::id(),
            NEXT_SAVE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|error| error.to_string())?;
        let result = file
            .write_all(&bytes)
            .and_then(|()| file.sync_all())
            .and_then(|()| fs::rename(&temp, &self.path));
        if let Err(error) = result {
            let _ = fs::remove_file(&temp);
            return Err(format!("Could not save configuration: {error}"));
        }
        self.original = Some(bytes);
        Ok(())
    }
}

fn validate(value: &Value) -> Result<(), String> {
    let object = value.as_object().ok_or("Expected a configuration object")?;
    if object.get("version").and_then(Value::as_u64) != Some(1) {
        return Err("Unsupported configuration version (expected 1)".into());
    }
    let plugins = object
        .get("plugins")
        .and_then(Value::as_object)
        .ok_or("Expected plugins object")?;
    for (id, plugin) in plugins {
        let plugin = plugin
            .as_object()
            .ok_or_else(|| format!("Expected plugin object: {id}"))?;
        if plugin
            .get("enabled")
            .is_some_and(|value| !value.is_boolean())
        {
            return Err(format!("Expected boolean enabled: {id}"));
        }
        if let Some(permissions) = plugin.get("permissions") {
            if !permissions
                .as_array()
                .is_some_and(|values| values.iter().all(Value::is_string))
            {
                return Err(format!("Expected permissions string array: {id}"));
            }
        }
        if plugin
            .get("settings")
            .is_some_and(|value| !value.is_object())
        {
            return Err(format!("Expected settings object: {id}"));
        }
    }
    if object
        .get("overrides")
        .is_some_and(|value| !value.is_object())
    {
        return Err("Expected overrides object".into());
    }
    Ok(())
}
