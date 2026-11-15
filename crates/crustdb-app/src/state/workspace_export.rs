//! Routing of `ExportDone` to the table tab or query tab that asked for the export.
use super::workspace::{AppState, Tab};
use crate::runtime::{Event, RequestId};

impl AppState {
    pub(super) fn export_done(&mut self, req: RequestId, result: Result<u64, String>) {
        let table = self
            .workspaces
            .iter_mut()
            .flat_map(|w| w.tabs.iter_mut())
            .map(|Tab::Table(t)| t)
            .find(|t| t.export_req == Some(req));
        if let Some(t) = table {
            t.finish_export(req, result);
        } else if let Some(q) = self
            .workspaces
            .iter_mut()
            .flat_map(|w| w.queries.iter_mut())
            .find(|q| q.pane.req == Some(req))
        {
            q.apply(&Event::ExportDone { req, result });
        }
    }
}

#[cfg(test)]
#[path = "workspace_export_tests.rs"]
mod tests;
