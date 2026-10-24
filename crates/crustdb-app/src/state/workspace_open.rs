//! Opening a workspace: a retry reuses the failed workspace of the same connection.
use super::*;

impl AppState {
    pub fn open_workspace(&mut self, config: ConnectionConfig) -> WorkspaceId {
        // ponytail: older duplicates of a failed connection (pre-#91 state) are left alone, only the
        // newest is reused; removing the rest here would leak their actors (no `Disconnect` is sent).
        let failed = self
            .workspaces
            .iter_mut()
            .rev()
            .find(|w| w.config.id == config.id && matches!(w.status, Status::Failed(_)));
        if let Some(w) = failed {
            let id = w.id;
            w.tree = ObjectTree::new(id, config.driver);
            w.config = config;
            w.status = Status::Connecting;
            // `Connect` drops this workspace's tab actors; reopen them (queued after it) so the kept
            // tabs and query texts keep working. Evaluations of the dead session never answer.
            for Tab::Table(t) in &w.tabs {
                self.outbox.push(Command::OpenTable {
                    ws: id,
                    tab: t.id,
                    object: t.object.clone(),
                });
            }
            for q in &mut w.queries {
                // The new actor starts with no transaction and no statement in flight.
                q.tx = crate::runtime::TxState::Idle;
                q.running = None;
                q.bg = false;
                self.outbox.push(Command::OpenEditor {
                    ws: id,
                    tab: q.id,
                    database: q.database.clone(),
                });
            }
            self.evaluating.retain(|_, (ws, _)| *ws != id);
            self.active = Some(id);
            return id;
        }
        let id = WorkspaceId(self.next_ws);
        self.next_ws += 1;
        self.workspaces.push(WorkspaceState {
            id,
            tree: ObjectTree::new(id, config.driver),
            columns: ColumnCache::default(),
            config,
            status: Status::Connecting,
            tabs: vec![],
            queries: vec![],
            active_tab: None,
            diagram: None,
            session_password: None,
        });
        self.active = Some(id);
        id
    }

    /// Newest workspace of `conn` that is connecting or online (a failed one may be retried).
    pub fn live_workspace(&self, conn: ConnectionId) -> Option<WorkspaceId> {
        self.workspaces
            .iter()
            .rev()
            .find(|w| w.config.id == conn && !matches!(w.status, Status::Failed(_)))
            .map(|w| w.id)
    }
}
