use super::{Descriptor, ProcessPlugin, MAX_MESSAGE, PROTOCOL_VERSION};
use crate::{Permission, Plugin, PLUGIN_API_VERSION};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
pub const MANIFEST_FILE: &str = "plugin.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub package_version: u32,
    pub protocol_version: u32,
    pub api_version: String,
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub environment: Vec<String>,
    #[serde(default)]
    pub credentials: Vec<String>,
    pub plugin: Descriptor,
}

impl Manifest {
    pub fn for_plugin(plugin: &dyn Plugin, args: Vec<String>) -> Self {
        Self {
            package_version: 1,
            protocol_version: PROTOCOL_VERSION,
            api_version: PLUGIN_API_VERSION.into(),
            executable: "plugin".into(),
            args,
            environment: Vec::new(),
            credentials: Vec::new(),
            plugin: Descriptor::from_plugin(plugin),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        self.plugin.validate()?;
        if self.executable.is_empty()
            || self.executable == MANIFEST_FILE
            || !self
                .executable
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
            || self.executable.starts_with('.')
        {
            return Err("Executable must be a regular filename inside the package".into());
        }
        for (names, permission) in [
            (&self.environment, Permission::Environment),
            (&self.credentials, Permission::Credentials),
        ] {
            if !names.is_empty() && !self.plugin.permissions.contains(&permission) {
                return Err(format!("Names require declared {permission:?} permission"));
            }
            if names
                .iter()
                .any(|name| name.is_empty() || name.contains(['=', '\0']))
            {
                return Err("Invalid environment name".into());
            }
        }
        Ok(())
    }
    pub fn compatible(&self) -> Result<(), String> {
        if self.package_version != 1
            || self.protocol_version != PROTOCOL_VERSION
            || self.api_version != PLUGIN_API_VERSION
        {
            return Err(format!(
                "Incompatible plugin: package {}, protocol {}, API {}; expected 1 / {} / {}",
                self.package_version,
                self.protocol_version,
                self.api_version,
                PROTOCOL_VERSION,
                PLUGIN_API_VERSION
            ));
        }
        Ok(())
    }
    pub fn read(directory: &Path) -> Result<Self, String> {
        let bytes = read_regular(&directory.join(MANIFEST_FILE), MAX_MESSAGE)?;
        let manifest: Self =
            serde_json::from_slice(&bytes).map_err(|e| format!("Invalid plugin manifest: {e}"))?;
        manifest.validate()?;
        Ok(manifest)
    }
}

pub(crate) fn regular(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!(
            "Expected regular file (no symlink): {}",
            path.display()
        ));
    }
    Ok(())
}
pub(crate) fn read_regular(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    regular(path)?;
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("Package file exceeds size limit".into());
    }
    Ok(bytes)
}

/// Creates a portable package from the same Plugin API used by the worker server.
pub fn write_package(
    plugin: &dyn Plugin,
    executable: &Path,
    args: Vec<String>,
    destination: &Path,
) -> Result<(), String> {
    let manifest = Manifest::for_plugin(plugin, args);
    write_parts(&manifest, executable, destination)
}
fn write_parts(manifest: &Manifest, executable: &Path, destination: &Path) -> Result<(), String> {
    manifest.validate()?;
    manifest.compatible()?;
    regular(executable)?;
    fs::create_dir(destination).map_err(|e| e.to_string())?;
    let result = (|| {
        let target = destination.join(&manifest.executable);
        fs::copy(executable, &target).map_err(|e| e.to_string())?;
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec_pretty(manifest).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_MESSAGE {
            return Err("Plugin manifest exceeds size limit".into());
        }
        fs::write(destination.join(MANIFEST_FILE), bytes).map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(destination);
    }
    result
}

#[derive(Clone)]
pub struct PackageStore {
    root: PathBuf,
}
impl PackageStore {
    pub fn new(root: PathBuf) -> Result<Self, String> {
        if !root.is_absolute() {
            return Err("Plugin installation directory must be absolute".into());
        }
        fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        if fs::symlink_metadata(&root)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            return Err("Plugin store must not be a symlink".into());
        }
        Ok(Self {
            root: root.canonicalize().map_err(|e| e.to_string())?,
        })
    }
    pub fn default_path() -> Result<PathBuf, String> {
        if let Some(path) = std::env::var_os("TW_PLUGIN_DIR") {
            return Ok(path.into());
        }
        let home = std::env::var_os("HOME").ok_or("HOME unavailable; set TW_PLUGIN_DIR")?;
        Ok(PathBuf::from(home).join(".local/share/terminal-workspace/plugins"))
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn directory(&self, id: &str) -> Result<PathBuf, String> {
        if id.is_empty()
            || id == "core"
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
        {
            return Err("Invalid plugin id".into());
        }
        let directory = self.root.join(id);
        if let Ok(metadata) = fs::symlink_metadata(&directory) {
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err("Installed package must be a directory, not a symlink".into());
            }
        }
        Ok(directory)
    }
    pub fn install(&self, source: &Path) -> Result<String, String> {
        let source = source.canonicalize().map_err(|e| e.to_string())?;
        let manifest = Manifest::read(&source)?;
        manifest.compatible()?;
        self.install_parts(&manifest, &source.join(&manifest.executable))?;
        Ok(manifest.plugin.id)
    }
    fn install_parts(&self, manifest: &Manifest, executable: &Path) -> Result<(), String> {
        let destination = self.directory(&manifest.plugin.id)?;
        if destination.exists() {
            return Err(format!("Plugin already installed: {}", manifest.plugin.id));
        }
        let temp = self.root.join(format!(
            ".install-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        write_parts(manifest, executable, &temp)?;
        if let Err(error) = fs::rename(&temp, &destination) {
            let _ = fs::remove_dir_all(&temp);
            return Err(error.to_string());
        }
        Ok(())
    }
    pub fn trust(&self, id: &str) -> Result<(), String> {
        let directory = self.directory(id)?;
        Manifest::read(&directory)?.compatible()?;
        let path = directory.join(".trusted");
        if fs::symlink_metadata(&path).is_ok() {
            regular(&path)?;
        }
        let temp = directory.join(format!(
            ".trust-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        let result = file
            .write_all(b"trusted-local-executable-v1\n")
            .and_then(|()| file.sync_all())
            .and_then(|()| fs::rename(&temp, path))
            .map_err(|e| e.to_string());
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }
    pub fn untrust(&self, id: &str) -> Result<(), String> {
        let path = self.directory(id)?.join(".trusted");
        if let Err(error) = fs::symlink_metadata(&path) {
            if error.kind() == std::io::ErrorKind::NotFound {
                return Ok(());
            }
            return Err(error.to_string());
        }
        regular(&path)?;
        fs::remove_file(path).map_err(|e| e.to_string())
    }
    pub fn is_trusted(&self, id: &str) -> bool {
        self.directory(id)
            .ok()
            .and_then(|dir| read_regular(&dir.join(".trusted"), 64).ok())
            .is_some_and(|bytes| bytes == b"trusted-local-executable-v1\n")
    }
    pub fn uninstall(&self, id: &str) -> Result<(), String> {
        let directory = self.directory(id)?;
        Manifest::read(&directory)?;
        let removed = self.root.join(format!(
            ".removed-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::rename(directory, &removed).map_err(|e| e.to_string())?;
        // Once renamed the package is no longer discoverable. Cleanup is best-effort.
        let _ = fs::remove_dir_all(removed);
        Ok(())
    }
    pub fn discover(&self) -> Result<Vec<Result<ProcessPlugin, String>>, String> {
        let mut entries: Vec<_> = fs::read_dir(&self.root)
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        entries.sort_by_key(|entry| entry.file_name());
        Ok(entries
            .into_iter()
            .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
            .map(|entry| {
                let id = entry.file_name().to_string_lossy().into_owned();
                self.load(&id).map_err(|error| format!("{id}: {error}"))
            })
            .collect())
    }
    pub fn load(&self, id: &str) -> Result<ProcessPlugin, String> {
        let directory = self.directory(id)?;
        let manifest = Manifest::read(&directory)?;
        if manifest.plugin.id != id {
            return Err("Package directory and plugin id differ".into());
        }
        Ok(ProcessPlugin::new(self.clone(), manifest))
    }
    /// Explicit distribution default, used once per store. Existing packages are never auto-trusted.
    pub fn bootstrap(
        &self,
        plugin: &dyn Plugin,
        executable: &Path,
        args: Vec<String>,
    ) -> Result<(), String> {
        let marker = self.root.join(".initialized");
        if fs::symlink_metadata(&marker).is_ok() {
            regular(&marker)?;
            return Ok(());
        }
        if !self.directory(plugin.id())?.exists() {
            self.install_parts(&Manifest::for_plugin(plugin, args), executable)?;
            self.trust(plugin.id())?;
        }
        use std::io::Write;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(marker)
            .and_then(|mut file| file.write_all(b"1\n"))
            .map_err(|e| e.to_string())
    }
}
