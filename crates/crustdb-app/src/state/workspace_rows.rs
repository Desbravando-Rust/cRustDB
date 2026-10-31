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
        if t.pending_blocks() || !t.writable() {
            return;
        }
        if let Some((lo, hi)) = t.selection() {
            self.dialogs.push(Dialog::DeleteRows {
                tab,
                count: hi - lo + 1,
            });
        }
    }

    /// Pops the dialog and sends the `DELETE` for the tab's current selection.
    pub fn confirm_delete_rows(&mut self, req: RequestId) -> Option<Command> {
        let Some(Dialog::DeleteRows { tab, .. }) = self.dialogs.pop() else {
            return None;
        };
        self.table_mut(tab)?.delete_selected(req)
    }
}

#[cfg(test)]
#[path = "workspace_rows_tests.rs"]
mod tests;
