//! Optional host polling contract. Executable plugins use the same wire operations.
use crate::Permission;
use crate::{Action, CommandInvocation, CommandOutcome, GroupView, Plugin, Workspace};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackgroundRequest {
    Start,
    View {
        group: String,
        location: Option<String>,
    },
    Refresh {
        group: String,
        location: Option<String>,
    },
    Actions {
        group: String,
        location: Option<String>,
        item: String,
    },
    Execute(CommandInvocation),
}
#[derive(Debug)]
pub enum BackgroundResponse {
    Started,
    View(GroupView),
    Refreshed(GroupView),
    Actions {
        view: GroupView,
        actions: Vec<Action>,
    },
    Executed(CommandOutcome),
}

pub(crate) fn run<P: Plugin + ?Sized>(
    plugin: &P,
    workspace: &Workspace,
    permissions: &BTreeSet<Permission>,
    request: &BackgroundRequest,
) -> Result<BackgroundResponse, String> {
    plugin.start(workspace, permissions)?;
    match request {
        BackgroundRequest::Start => Ok(BackgroundResponse::Started),
        BackgroundRequest::View { group, location } => plugin
            .view(workspace, group, location.as_deref())
            .map(BackgroundResponse::View),
        BackgroundRequest::Refresh { group, location } => plugin
            .refresh(workspace, group, location.as_deref())
            .map(BackgroundResponse::Refreshed),
        BackgroundRequest::Actions {
            group,
            location,
            item,
        } => {
            let view = plugin.view(workspace, group, location.as_deref())?;
            let selected = view
                .items
                .iter()
                .find(|entry| &entry.id == item)
                .ok_or_else(|| format!("Item not found: {item}"))?;
            let actions = plugin.try_actions(selected)?;
            Ok(BackgroundResponse::Actions { view, actions })
        }
        BackgroundRequest::Execute(invocation) => plugin
            .execute(workspace, invocation)
            .map(BackgroundResponse::Executed),
    }
}
