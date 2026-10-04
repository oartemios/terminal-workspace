use super::package::regular;
use super::{
    Availability, Connection, Context, Manifest, Operation, PackageStore, Reply, Request,
    RuntimeStatus, ViewReply, MAX_MESSAGE, PROTOCOL_VERSION, REQUEST_TIMEOUT_MS,
};
use crate::{
    Action, Command, CommandInvocation, CommandOutcome, Group, GroupView, Item, KeyBinding,
    Permission, Plugin, Workspace,
};
use serde::de::DeserializeOwned;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdin, ChildStdout, Command as Process, Stdio};
use std::time::{Duration, Instant};

struct Worker {
    child: Child,
    input: ChildStdin,
    output: ChildStdout,
    bytes: Vec<u8>,
    next_id: u64,
}
impl Worker {
    fn spawn(
        store: &PackageStore,
        manifest: &Manifest,
        permissions: &BTreeSet<Permission>,
    ) -> Result<Self, String> {
        let directory = store.directory(&manifest.plugin.id)?;
        let executable = directory.join(&manifest.executable);
        regular(&executable)?;
        let mut command = Process::new(executable);
        command
            .args(&manifest.args)
            .current_dir(directory)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for (names, permission) in [
            (&manifest.environment, Permission::Environment),
            (&manifest.credentials, Permission::Credentials),
        ] {
            if permissions.contains(&permission) {
                for name in names {
                    if let Some(value) = std::env::var_os(name) {
                        command.env(name, value);
                    }
                }
            }
        }
        // SAFETY: the pre-exec callback uses only the async-signal-safe setsid syscall.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("Cannot start plugin: {e}"))?;
        let input = child.stdin.take().ok_or("Missing plugin stdin")?;
        let output = child.stdout.take().ok_or("Missing plugin stdout")?;
        let worker = Self {
            child,
            input,
            output,
            bytes: Vec::new(),
            next_id: 0,
        };
        nonblocking(worker.input.as_raw_fd())?;
        nonblocking(worker.output.as_raw_fd())?;
        Ok(worker)
    }
    fn call<T: DeserializeOwned>(&mut self, body: Operation) -> Result<Result<T, String>, String> {
        self.next_id += 1;
        let timeout = if matches!(&body, Operation::Describe) {
            super::STARTUP_TIMEOUT_MS
        } else {
            REQUEST_TIMEOUT_MS
        };
        let request = Request {
            protocol: PROTOCOL_VERSION,
            request_id: self.next_id,
            body,
        };
        let mut bytes = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
        if bytes.len() >= MAX_MESSAGE {
            return Err("Plugin request exceeds size limit".into());
        }
        bytes.push(b'\n');
        let deadline = Instant::now() + Duration::from_millis(timeout);
        let mut written = 0;
        while written < bytes.len() {
            match self.input.write(&bytes[written..]) {
                Ok(0) => return Err("Plugin stdin closed".into()),
                Ok(count) => written += count,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    wait_fd(self.input.as_raw_fd(), libc::POLLOUT, deadline)?
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(format!("Plugin write failed: {error}")),
            }
            if Instant::now() >= deadline {
                return Err("Plugin request timed out".into());
            }
        }
        loop {
            if let Some(end) = self.bytes.iter().position(|byte| *byte == b'\n') {
                let tail = self.bytes.split_off(end + 1);
                let reply: Reply = serde_json::from_slice(&self.bytes[..end])
                    .map_err(|e| format!("Invalid plugin response: {e}"))?;
                self.bytes = tail;
                if reply.protocol != PROTOCOL_VERSION || reply.request_id != request.request_id {
                    return Err("Plugin response protocol/id mismatch".into());
                }
                return match reply.result {
                    Err(error) => Ok(Err(error)),
                    Ok(value) => serde_json::from_value(value)
                        .map(Ok)
                        .map_err(|e| format!("Invalid plugin result: {e}")),
                };
            }
            if Instant::now() >= deadline {
                return Err("Plugin request timed out".into());
            }
            let mut chunk = [0u8; 4096];
            match self.output.read(&mut chunk) {
                Ok(0) => return Err("Plugin process closed its output".into()),
                Ok(count) => {
                    self.bytes.extend_from_slice(&chunk[..count]);
                    if self.bytes.len() > MAX_MESSAGE {
                        return Err("Plugin response exceeds size limit".into());
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    wait_fd(self.output.as_raw_fd(), libc::POLLIN, deadline)?
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(format!("Plugin read failed: {error}")),
            }
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        // SAFETY: this worker creates its own process group with its child pid.
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn nonblocking(fd: RawFd) -> Result<(), String> {
    // SAFETY: fd belongs to the live child pipe; fcntl does not retain a pointer.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}
fn wait_fd(fd: RawFd, events: i16, deadline: Instant) -> Result<(), String> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("Plugin request timed out".into());
        }
        let mut descriptor = libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        // SAFETY: descriptor is one initialized pollfd valid for this syscall.
        let ready = unsafe {
            libc::poll(
                &mut descriptor,
                1,
                remaining.as_millis().max(1).min(i32::MAX as u128) as i32,
            )
        };
        if ready > 0 {
            if descriptor.revents & events != 0 {
                return Ok(());
            }
            return Err("Plugin pipe disconnected".into());
        }
        if ready == 0 {
            return Err("Plugin request timed out".into());
        }
        if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return Err(std::io::Error::last_os_error().to_string());
        }
    }
}

#[derive(Default)]
struct State {
    worker: Option<Worker>,
    context: Option<Context>,
    error: Option<String>,
    icons: BTreeMap<String, char>,
}
pub struct ProcessPlugin {
    store: PackageStore,
    manifest: Manifest,
    state: RefCell<State>,
}
impl ProcessPlugin {
    pub(crate) fn new(store: PackageStore, manifest: Manifest) -> Self {
        Self {
            store,
            manifest,
            state: RefCell::new(State::default()),
        }
    }
    fn call<T: DeserializeOwned>(&self, operation: Operation) -> Result<T, String> {
        let mut state = self.state.borrow_mut();
        let result = state
            .worker
            .as_mut()
            .ok_or("Plugin is disconnected")?
            .call(operation);
        if let Err(error) = &result {
            state.worker.take();
            state.error = Some(error.clone());
            state.icons.clear();
        }
        result.and_then(|result| result)
    }
    fn context(&self) -> Result<Context, String> {
        self.state
            .borrow()
            .context
            .clone()
            .ok_or_else(|| "Plugin is disconnected".into())
    }
}
impl Plugin for ProcessPlugin {
    fn id(&self) -> &str {
        &self.manifest.plugin.id
    }
    fn name(&self) -> &str {
        &self.manifest.plugin.name
    }
    fn permissions(&self) -> Vec<Permission> {
        self.manifest.plugin.permissions.clone()
    }
    fn groups(&self) -> Vec<Group> {
        self.manifest.plugin.groups.clone()
    }
    fn commands(&self) -> Vec<Command> {
        self.manifest.plugin.commands.clone()
    }
    fn keybindings(&self) -> Vec<KeyBinding> {
        self.manifest.plugin.keybindings.clone()
    }
    fn start(
        &self,
        workspace: &Workspace,
        permissions: &BTreeSet<Permission>,
    ) -> Result<(), String> {
        self.manifest.compatible()?;
        if !self.store.is_trusted(self.id()) {
            self.stop();
            return Err(format!(
                "Plugin {} is untrusted; use core.plugin.trust before local execution",
                self.id()
            ));
        }
        for permission in &self.manifest.plugin.permissions {
            if !permissions.contains(permission) {
                return Err(format!(
                    "Permission {permission:?} not granted to {}",
                    self.id()
                ));
            }
        }
        let context = Context::new(workspace, self.id(), permissions);
        {
            let state = self.state.borrow();
            if let Some(error) = &state.error {
                return Err(format!(
                    "{}: {error}; enable or restart to retry",
                    self.id()
                ));
            }
            if state.worker.is_some() && state.context.as_ref() == Some(&context) {
                return Ok(());
            }
        }
        self.stop();
        let result: Result<(), String> = (|| {
            let mut worker = Worker::spawn(&self.store, &self.manifest, permissions)?;
            let descriptor: super::Descriptor = worker
                .call(Operation::Describe)
                .map_err(|error| format!("Plugin handshake failed: {error}"))??;
            if descriptor != self.manifest.plugin {
                return Err("Plugin handshake differs from installed manifest".into());
            }
            let mut state = self.state.borrow_mut();
            state.worker = Some(worker);
            state.context = Some(context);
            Ok(())
        })();
        if let Err(error) = &result {
            self.state.borrow_mut().error = Some(error.clone());
        }
        result
    }
    fn stop(&self) {
        *self.state.borrow_mut() = State::default();
    }
    fn runtime_status(&self) -> RuntimeStatus {
        let mut state = self.state.borrow_mut();
        let mut availability = if self.manifest.compatible().is_err() {
            Availability::Incompatible
        } else if self
            .store
            .directory(self.id())
            .ok()
            .and_then(|dir| regular(&dir.join(&self.manifest.executable)).ok())
            .is_none()
        {
            Availability::Missing
        } else if !self.store.is_trusted(self.id()) {
            Availability::Untrusted
        } else if state.error.is_some() {
            Availability::Failed
        } else {
            Availability::Available
        };
        if let Some(worker) = &mut state.worker {
            if let Ok(Some(status)) = worker.child.try_wait() {
                state.error = Some(format!("Plugin process exited: {status}"));
                state.worker.take();
                if availability == Availability::Available {
                    availability = Availability::Failed;
                }
            }
        }
        if matches!(
            availability,
            Availability::Missing | Availability::Untrusted | Availability::Incompatible
        ) {
            state.worker.take();
            state.context = None;
            state.icons.clear();
        }
        RuntimeStatus {
            availability,
            connection: if state.worker.is_some() {
                Connection::Running
            } else if state.error.is_some() {
                Connection::Failed
            } else {
                Connection::Disconnected
            },
            detail: self
                .manifest
                .compatible()
                .err()
                .or_else(|| state.error.clone()),
        }
    }
    fn installation_present(&self) -> bool {
        self.store
            .directory(self.id())
            .ok()
            .and_then(|dir| regular(&dir.join("plugin.json")).ok())
            .is_some()
    }
    fn items(&self, workspace: &Workspace, group: &str) -> Result<Vec<Item>, String> {
        Ok(self.view(workspace, group, None)?.items)
    }
    fn view(
        &self,
        _workspace: &Workspace,
        group: &str,
        location: Option<&str>,
    ) -> Result<GroupView, String> {
        let reply: ViewReply = self.call(Operation::View {
            context: self.context()?,
            group: group.into(),
            location: location.map(str::to_owned),
        })?;
        self.state.borrow_mut().icons = reply.icons;
        Ok(reply.view)
    }
    fn item_icon(&self, item: &Item) -> char {
        self.state
            .borrow()
            .icons
            .get(&item.id)
            .copied()
            .unwrap_or('•')
    }
    fn actions(&self, item: &Item) -> Vec<Action> {
        self.try_actions(item).unwrap_or_default()
    }
    fn try_actions(&self, item: &Item) -> Result<Vec<Action>, String> {
        self.call(Operation::Actions {
            context: self.context()?,
            item: item.clone(),
        })
    }
    fn execute(
        &self,
        _workspace: &Workspace,
        invocation: &CommandInvocation,
    ) -> Result<CommandOutcome, String> {
        self.call(Operation::Execute {
            context: self.context()?,
            invocation: invocation.clone(),
        })
    }
}
