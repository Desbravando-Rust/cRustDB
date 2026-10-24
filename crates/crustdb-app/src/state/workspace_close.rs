use super::workspace::AppState;
use crate::runtime::WorkspaceId;

impl AppState {
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
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crate::runtime::RequestId;
    use crate::state::Dialog;
    use crate::state::workspace::tests::state;

    #[test]
    fn close_workspace_clears_evaluating_dialogs_and_active() {
        // Arrange
        let mut s = state(&["a", "b"]);
        let a = s.open_workspace(s.config.connections[0].clone());
        let b = s.open_workspace(s.config.connections[1].clone());
        s.open_query(a, None);
        let tab_a = s.workspaces[0].queries[0].id;
        s.open_query(b, None);
        let tab_b = s.workspaces[1].queries[0].id;
        s.active = Some(a);
        s.register_evaluate(RequestId(1), a, tab_a);
        s.register_evaluate(RequestId(2), b, tab_b);
        s.dialogs.push(Dialog::CloseWithTx { tab: tab_a });
        s.dialogs.push(Dialog::CloseWithTx { tab: tab_b });
        s.dialogs.push(Dialog::HostKey {
            ws: a,
            host: "h".into(),
            fingerprint: "f".into(),
        });

        // Act
        s.close_workspace(a);

        // Assert
        assert_eq!(s.workspaces.len(), 1);
        assert_eq!(s.active, None);
        assert_eq!(s.evaluating.len(), 1);
        assert!(s.evaluating.contains_key(&RequestId(2)));
        assert!(matches!(s.dialogs.top(), Some(Dialog::CloseWithTx { tab }) if *tab == tab_b));
        s.dialogs.pop();
        assert!(s.dialogs.top().is_none());
    }
}
