//! Confirming the deletion of the selected rows of a table tab (ADR 0010).
use super::dialogs::Dialog;
use super::workspace::AppState;
use crate::runtime::{Command, RequestId, TabId};

impl AppState {
    /// Asks for confirmation when the tab can delete its selection; a refusal shows a notice.
    pub fn request_delete_rows(&mut self, tab: TabId) {
        let Some(t) = self.table_mut(tab) else {
            return;
        };
        if t.pending_blocks() || !t.writable() || t.delete_busy() {
            return;
        }
        if let Some((lo, hi)) = t.selection() {
            self.dialogs.push(Dialog::DeleteRows {
                tab,
                range: (lo, hi),
                count: hi - lo + 1,
            });
        }
    }

    /// The open DeleteRows dialog's table is loading or saving: its Delete button is disabled.
    pub fn delete_rows_busy(&mut self) -> bool {
        let Some(&Dialog::DeleteRows { tab, .. }) = self.dialogs.top() else {
            return false;
        };
        self.table_mut(tab)
            .is_some_and(|t| t.loading.is_some() || t.edit_req.is_some())
    }

    /// Pops the dialog and sends the `DELETE` for the rows the dialog counted; if the selection
    /// moved meanwhile (the dialog is not modal), nothing is sent.
    pub fn confirm_delete_rows(&mut self, req: RequestId) -> Option<Command> {
        let Some(Dialog::DeleteRows { tab, range, .. }) = self.dialogs.pop() else {
            return None;
        };
        let t = self.table_mut(tab)?;
        if t.selection() != Some(range) {
            t.notice = Some("Selection changed, delete cancelled".into());
            return None;
        }
        t.delete_selected(req)
    }
}

#[cfg(test)]
#[path = "workspace_rows_tests.rs"]
mod tests;
