//! Read-only Git integration, available through the ordinary executable plugin API.
use crate::{
    Action, BindingScope, Block, Command, CommandInvocation, CommandOutcome, Group, Item,
    KeyBinding, Permission, Plugin, Workspace,
};
use std::io::Read;
use std::path::{Component, Path};
use std::process::{Command as Process, Stdio};

pub struct GitPlugin;
const OUTPUT_LIMIT: usize = 128 * 1024;

impl Plugin for GitPlugin {
    fn id(&self) -> &str {
        "git"
    }
    fn name(&self) -> &str {
        "Git"
    }
    fn permissions(&self) -> Vec<Permission> {
        vec![Permission::WorkspaceRead, Permission::Process]
    }
    fn groups(&self) -> Vec<Group> {
        vec![
            Group {
                id: "status".into(),
                title: "Status".into(),
            },
            Group {
                id: "branches".into(),
                title: "Branches".into(),
            },
        ]
    }
    fn items(&self, workspace: &Workspace, group: &str) -> Result<Vec<Item>, String> {
        repository(workspace)?;
        match group {
            "status" => Ok(status(workspace)?
                .into_iter()
                .map(|entry| Item {
                    id: entry.path.clone(),
                    title: format!("{} {}", entry.xy, entry.display()),
                    kind: "changed-file".into(),
                })
                .collect()),
            "branches" => branches(workspace),
            _ => Err(format!("Unknown Git group: {group}")),
        }
    }
    fn actions(&self, item: &Item) -> Vec<Action> {
        let (command_id, label) = match item.kind.as_str() {
            "changed-file" => ("git.diff", "View file diff"),
            "branch" => ("git.branch", "View branch details"),
            _ => return Vec::new(),
        };
        vec![Action {
            label: label.into(),
            command_id: command_id.into(),
            is_default: true,
            invocation: CommandInvocation {
                id: command_id.into(),
                item: Some(item.id.clone()),
                args: Vec::new(),
            },
        }]
    }
    fn commands(&self) -> Vec<Command> {
        [
            ("git.diff", "View file diff"),
            ("git.branch", "View branch details"),
        ]
        .into_iter()
        .map(|(id, title)| Command {
            id: id.into(),
            title: title.into(),
            requires_item: true,
        })
        .collect()
    }
    fn keybindings(&self) -> Vec<KeyBinding> {
        [("status", "git.diff"), ("branches", "git.branch")]
            .into_iter()
            .map(|(group, command)| KeyBinding {
                keys: "p".into(),
                command_id: command.into(),
                scope: BindingScope::Group {
                    plugin: "git".into(),
                    group: group.into(),
                },
            })
            .collect()
    }
    fn execute(
        &self,
        workspace: &Workspace,
        invocation: &CommandInvocation,
    ) -> Result<CommandOutcome, String> {
        repository(workspace)?;
        let item = invocation.item.as_deref().ok_or("Missing Git item")?;
        let content = match invocation.id.as_str() {
            "git.diff" => file_diff(workspace, item)?,
            "git.branch" => {
                if !branches(workspace)?.iter().any(|branch| branch.id == item) {
                    return Err("Branch no longer exists; refresh Branches".into());
                }
                let detail = run(
                    workspace,
                    &[
                        "log",
                        "-1",
                        "--format=%H%nAuthor: %an%nDate: %aI%n%n%s",
                        item,
                        "--",
                    ],
                    false,
                )?;
                format!(
                    "Branch: {}\n\n{}",
                    item.trim_start_matches("refs/heads/"),
                    utf8(detail)?
                )
            }
            _ => return Err(format!("Unknown Git command: {}", invocation.id)),
        };
        Ok(CommandOutcome::Output(Block {
            format: crate::ContentFormat::Text,
            source: "Git".into(),
            status: "ok".into(),
            content,
        }))
    }
}

struct Change {
    xy: String,
    path: String,
    original: Option<String>,
}
impl Change {
    fn display(&self) -> String {
        self.original
            .as_ref()
            .map_or_else(|| self.path.clone(), |old| format!("{old} → {}", self.path))
    }
}
fn status(workspace: &Workspace) -> Result<Vec<Change>, String> {
    let output = run(
        workspace,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ],
        false,
    )?;
    let mut records = output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty());
    let mut changes = Vec::new();
    while let Some(record) = records.next() {
        if record.len() < 4 || record[2] != b' ' {
            return Err("Invalid Git status response".into());
        }
        let original = if record[..2].contains(&b'R') || record[..2].contains(&b'C') {
            Some(utf8(
                records.next().ok_or("Missing rename source")?.to_vec(),
            )?)
        } else {
            None
        };
        changes.push(Change {
            xy: utf8(record[..2].to_vec())?,
            path: utf8(record[3..].to_vec())?,
            original,
        });
    }
    Ok(changes)
}
fn branches(workspace: &Workspace) -> Result<Vec<Item>, String> {
    let output = utf8(run(
        workspace,
        &[
            "for-each-ref",
            "--sort=refname",
            "--format=%(HEAD)%09%(refname)",
            "refs/heads/",
        ],
        false,
    )?)?;
    output
        .lines()
        .map(|line| {
            let (current, name) = line.split_once('\t').ok_or("Invalid Git branch response")?;
            Ok(Item {
                id: name.into(),
                title: format!("{} {}", current, name.trim_start_matches("refs/heads/")),
                kind: "branch".into(),
            })
        })
        .collect()
}
fn repository(workspace: &Workspace) -> Result<(), String> {
    workspace.read_path(".")?;
    let root = utf8(run(workspace, &["rev-parse", "--show-toplevel"], false)?)?;
    let root = Path::new(root.trim_end_matches('\n'))
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if root != workspace.root() {
        return Err("Open the repository root as Workspace to use Git".into());
    }
    Ok(())
}
fn file_diff(workspace: &Workspace, path: &str) -> Result<String, String> {
    if Path::new(path)
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err("Git file path must be relative to the repository root".into());
    }
    let entry = status(workspace)?
        .into_iter()
        .find(|entry| entry.path == path)
        .ok_or("File has no changes; refresh Status")?;
    if entry.xy == "??" {
        // This also prevents following an untracked symlink outside the Workspace.
        workspace.read_path(path)?;
        let diff = run(
            workspace,
            &[
                "diff",
                "--no-index",
                "--no-ext-diff",
                "--no-textconv",
                "--",
                "/dev/null",
                path,
            ],
            true,
        )?;
        return Ok(format!("Untracked: {path}\n\n{}", utf8(diff)?));
    }
    let mut paths = vec![path];
    if let Some(original) = entry.original.as_deref() {
        paths.push(original);
    }
    let mut sections = Vec::new();
    for (title, cached) in [
        ("Staged (HEAD → index)", true),
        ("Unstaged (index → working tree)", false),
    ] {
        let mut args = vec!["diff", "--no-ext-diff", "--no-textconv"];
        if cached {
            args.push("--cached");
        }
        args.push("--");
        args.extend(&paths);
        let diff = utf8(run(workspace, &args, false)?)?;
        if !diff.is_empty() {
            sections.push(format!("{title}\n\n{diff}"));
        }
    }
    if sections.is_empty() {
        Ok("No textual diff (file changed or submodule has local changes); refresh Status".into())
    } else {
        Ok(sections.join("\n"))
    }
}
fn utf8(bytes: Vec<u8>) -> Result<String, String> {
    String::from_utf8(bytes).map_err(|_| {
        "Git output is not valid UTF-8 (binary paths or content are unsupported)".into()
    })
}
fn read_bounded(reader: impl Read) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(OUTPUT_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}
fn run(workspace: &Workspace, args: &[&str], allow_difference: bool) -> Result<Vec<u8>, String> {
    let mut child = Process::new("git")
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_LITERAL_PATHSPECS", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args([
            "--no-pager",
            "-c",
            "color.ui=false",
            "-c",
            "core.fsmonitor=false",
        ])
        .args(args)
        .current_dir(workspace.root())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Cannot start Git: {error}"))?;
    let stdout = child.stdout.take().ok_or("Missing Git stdout")?;
    let stderr = child.stderr.take().ok_or("Missing Git stderr")?;
    let out = std::thread::spawn(move || read_bounded(stdout));
    let err = std::thread::spawn(move || read_bounded(stderr));
    let status = child.wait().map_err(|error| error.to_string())?;
    let output = out.join().map_err(|_| "Git output reader failed")??;
    let error = err.join().map_err(|_| "Git error reader failed")??;
    if output.len() > OUTPUT_LIMIT || error.len() > OUTPUT_LIMIT {
        return Err("Git output exceeds 128 KiB limit".into());
    }
    if !(status.success() || allow_difference && status.code() == Some(1)) {
        return Err(format!("Git: {}", String::from_utf8_lossy(&error).trim()));
    }
    Ok(output)
}
