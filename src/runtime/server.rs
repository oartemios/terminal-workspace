use super::{
    Context, Descriptor, Operation, Reply, Request, ViewReply, MAX_MESSAGE, PROTOCOL_VERSION,
};
use crate::{CommandOutcome, Plugin};
use std::io::{self, BufRead, Read, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};

pub fn serve_plugin(plugin: &dyn Plugin) -> Result<(), String> {
    let descriptor = Descriptor::from_plugin(plugin);
    descriptor.validate()?;
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    loop {
        let mut bytes = Vec::new();
        let count = input
            .by_ref()
            .take(MAX_MESSAGE as u64 + 1)
            .read_until(b'\n', &mut bytes)
            .map_err(|e| e.to_string())?;
        if count == 0 {
            return Ok(());
        }
        if count > MAX_MESSAGE || bytes.last() != Some(&b'\n') {
            return Err("Invalid or oversized request".into());
        }
        let request: Request = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let mut crashed = false;
        let result = if request.protocol != PROTOCOL_VERSION {
            Err("Incompatible protocol".into())
        } else {
            catch_unwind(AssertUnwindSafe(|| {
                dispatch(plugin, &descriptor, request.body)
            }))
            .unwrap_or_else(|_| {
                crashed = true;
                Err("Plugin panicked".into())
            })
        };
        let reply = Reply {
            protocol: PROTOCOL_VERSION,
            request_id: request.request_id,
            result,
        };
        let mut bytes = serde_json::to_vec(&reply).map_err(|e| e.to_string())?;
        if bytes.len() >= MAX_MESSAGE {
            bytes = serde_json::to_vec(&Reply {
                protocol: PROTOCOL_VERSION,
                request_id: request.request_id,
                result: Err("Response exceeds size limit".into()),
            })
            .map_err(|e| e.to_string())?;
        }
        bytes.push(b'\n');
        output
            .write_all(&bytes)
            .and_then(|()| output.flush())
            .map_err(|e| e.to_string())?;
        if crashed {
            return Err("Plugin panicked".into());
        }
    }
}
fn authorize(context: &Context, descriptor: &Descriptor) -> Result<(), String> {
    for permission in &descriptor.permissions {
        if !context.permissions.contains(permission) {
            return Err(format!("Permission {permission:?} not granted"));
        }
    }
    Ok(())
}
fn dispatch(
    plugin: &dyn Plugin,
    descriptor: &Descriptor,
    operation: Operation,
) -> Result<serde_json::Value, String> {
    match operation {
        Operation::Describe => serde_json::to_value(descriptor).map_err(|e| e.to_string()),
        Operation::View {
            context,
            group,
            location,
        } => {
            authorize(&context, descriptor)?;
            let workspace = context.workspace(plugin.id())?;
            plugin.start(&workspace, &context.permissions)?;
            let view = plugin.view(&workspace, &group, location.as_deref())?;
            let icons = view
                .items
                .iter()
                .map(|item| (item.id.clone(), plugin.item_icon(item)))
                .collect();
            serde_json::to_value(ViewReply { view, icons }).map_err(|e| e.to_string())
        }
        Operation::Actions { context, item } => {
            authorize(&context, descriptor)?;
            plugin.start(&context.workspace(plugin.id())?, &context.permissions)?;
            serde_json::to_value(plugin.try_actions(&item)?).map_err(|e| e.to_string())
        }
        Operation::Execute {
            context,
            invocation,
        } => {
            authorize(&context, descriptor)?;
            if !descriptor
                .commands
                .iter()
                .any(|command| command.id == invocation.id)
            {
                return Err("Unknown plugin command".into());
            }
            let workspace = context.workspace(plugin.id())?;
            plugin.start(&workspace, &context.permissions)?;
            let outcome = plugin.execute(&workspace, &invocation)?;
            if matches!(outcome, CommandOutcome::WorkspaceChanged) {
                return Err("WorkspaceChanged is reserved for Core".into());
            }
            serde_json::to_value(outcome).map_err(|e| e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Action, Command, CommandInvocation, Group, Item, Workspace};
    use std::collections::BTreeSet;

    struct UnavailableActions;
    impl Plugin for UnavailableActions {
        fn id(&self) -> &str {
            "probe"
        }
        fn name(&self) -> &str {
            "Probe"
        }
        fn groups(&self) -> Vec<Group> {
            Vec::new()
        }
        fn items(&self, _: &Workspace, _: &str) -> Result<Vec<Item>, String> {
            Ok(Vec::new())
        }
        fn actions(&self, _: &Item) -> Vec<Action> {
            panic!("fallible override must be used")
        }
        fn try_actions(&self, _: &Item) -> Result<Vec<Action>, String> {
            Err("Actions temporarily unavailable".into())
        }
        fn commands(&self) -> Vec<Command> {
            Vec::new()
        }
        fn execute(&self, _: &Workspace, _: &CommandInvocation) -> Result<CommandOutcome, String> {
            unreachable!()
        }
    }

    #[test]
    fn sdk_preserves_fallible_action_errors() {
        let plugin = UnavailableActions;
        let workspace = Workspace::open(std::env::current_dir().unwrap()).unwrap();
        let result = dispatch(
            &plugin,
            &Descriptor::from_plugin(&plugin),
            Operation::Actions {
                context: Context::new(&workspace, "probe", &BTreeSet::new()),
                item: Item {
                    id: "note".into(),
                    title: "Note".into(),
                    kind: "note".into(),
                },
            },
        );
        assert_eq!(result.unwrap_err(), "Actions temporarily unavailable");
    }
}
