use super::dialogs::Dialog;
use super::workspace::{AppState, Tab};
use crate::runtime::{Command, TabId, TxOp, TxState, WorkspaceId};

impl AppState {
    /// Disconnects straight away, or asks first when a transaction or row edits would be lost.
    pub fn request_disconnect(&mut self, ws: WorkspaceId) -> Vec<Command> {
        if self.ask_disconnect(ws, false) {
            return vec![];
        }
        self.close_workspace(ws);
        vec![Command::Disconnect { ws }]
    }

    /// Opens the disconnect dialog when something would be lost; returns whether it did.
    /// `busy`: a sibling query is mid-flight, so its `BEGIN` could still open a transaction.
    fn ask_disconnect(&mut self, ws: WorkspaceId, busy: bool) -> bool {
        let has_tx = busy || !self.tx_tabs(ws).is_empty();
        let pending_rows = self
            .workspaces
            .iter()
            .filter(|w| w.id == ws)
            .flat_map(|w| &w.tabs)
            .filter(|Tab::Table(t)| t.has_pending())
            .count();
        if !has_tx && pending_rows == 0 {
            return false;
        }
        self.dialogs.push(Dialog::Disconnect {
            ws,
            has_tx,
            pending_rows,
        });
        true
    }

    /// Commit or roll back every open transaction, then disconnect. `Commit`/`Rollback` only.
    pub fn resolve_disconnect(&mut self, op: TxOp) -> Vec<Command> {
        let Some(Dialog::Disconnect { ws, .. }) = self.dialogs.top() else {
            return vec![];
        };
        let ws = *ws;
        if !matches!(op, TxOp::Commit | TxOp::Rollback) {
            return vec![];
        }
        self.dialogs.pop();
        let mut cmds: Vec<Command> = self
            .tx_tabs(ws)
            .into_iter()
            .map(|tab| Command::Tx { tab, op })
            .collect();
        if cmds.is_empty() {
            self.close_workspace(ws);
            cmds.push(Command::Disconnect { ws });
        } else {
            // The real Disconnect waits for every TxChanged{Idle} (see `tx_resolved`).
            self.disconnecting
                .insert(ws, cmds.iter().filter_map(tx_tab).collect());
        }
        cmds
    }

    /// An editor finished its transaction; once the last one of a disconnecting workspace did,
    /// the workspace goes away and `Disconnect` is queued.
    pub(super) fn tx_resolved(&mut self, tab: TabId) {
        let Some((&ws, _)) = self.disconnecting.iter().find(|(_, t)| t.contains(&tab)) else {
            return;
        };
        let tabs = self.disconnecting.entry(ws).or_default();
        tabs.remove(&tab);
        if !tabs.is_empty() {
            return;
        }
        self.disconnecting.remove(&ws);
        // Something new may have appeared while resolving: ask again instead of dropping it.
        let busy = self
            .workspaces
            .iter()
            .filter(|w| w.id == ws)
            .flat_map(|w| &w.queries)
            .any(|q| q.running.is_some() || self.evaluating.values().any(|(_, t)| *t == q.id));
        if !self.ask_disconnect(ws, busy) {
            self.close_workspace(ws);
            self.outbox.push(Command::Disconnect { ws });
        }
    }

    fn tx_tabs(&self, ws: WorkspaceId) -> Vec<TabId> {
        self.workspaces
            .iter()
            .filter(|w| w.id == ws)
            .flat_map(|w| &w.queries)
            .filter(|q| q.tx != TxState::Idle)
            .map(|q| q.id)
            .collect()
    }

    /// Forgets a workspace and everything that still points at it.
    pub fn close_workspace(&mut self, ws: WorkspaceId) {
        // Tabs must be read before the workspace goes away.
        let tabs = self
            .workspaces
            .iter()
            .find(|w| w.id == ws)
            .map(|w| w.tab_ids())
            .unwrap_or_default();
        self.dialogs.retain_ws(ws, &tabs);
        self.workspaces.retain(|w| w.id != ws);
        if self.active == Some(ws) {
            self.active = None;
        }
        self.evaluating.retain(|_, (w, _)| *w != ws);
        self.disconnecting.remove(&ws);
    }
}

fn tx_tab(c: &Command) -> Option<TabId> {
    match c {
        Command::Tx { tab, .. } => Some(*tab),
        _ => None,
    }
}

#[cfg(test)]
#[path = "workspace_close_tests.rs"]
mod tests;
