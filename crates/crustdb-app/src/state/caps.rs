//! Capability gate: which screens and actions a workspace's data model supports.
use crustdb_core::model::Capabilities;

use super::WorkspaceState;

impl WorkspaceState {
    /// The single source for every "is this action available" decision.
    pub fn capabilities(&self) -> Capabilities {
        self.config.driver.data_model().capabilities()
    }

    /// The object tree, or `None` when the data model has no table tab (nothing to browse).
    pub fn tree_if_supported(&mut self) -> Option<&mut super::tree::ObjectTree> {
        self.capabilities().table_tab.then_some(&mut self.tree)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use crustdb_core::model::{DriverKind, ObjectKind, ObjectRef, Product, ServerVersion};

    use crate::runtime::{Command, Event, RequestId};
    use crate::state::AppState;
    use crate::state::workspace::tests::{conn, state};

    fn online_redis() -> (AppState, crate::runtime::WorkspaceId) {
        let mut s = state(&[]);
        let mut c = conn("r");
        c.driver = DriverKind::Redis;
        c.database = Some("0".into());
        let ws = s.open_workspace(c);
        s.apply(Event::Connected {
            ws,
            server: ServerVersion {
                product: Product::Redis,
                major: 7,
                minor: 0,
                patch: 0,
                raw: "7.0.0".into(),
            },
        });
        (s, ws)
    }

    #[test]
    fn redis_connection_disables_table_diagram_export_actions() {
        // Arrange
        let (mut s, ws) = online_redis();
        let object = ObjectRef {
            database: "0".into(),
            schema: None,
            name: "k".into(),
            kind: ObjectKind::Table,
        };

        // Act
        let table = s.open_table(ws, object, RequestId(1));
        let query = s.open_query(ws, None);
        let w = s.workspace_mut(ws).unwrap();
        let diagram = w.open_diagram(RequestId(2), "0".into(), None);
        let home = w.expand_home();

        // Assert
        assert!(table.is_empty());
        assert!(query.is_none());
        assert!(diagram.is_none());
        assert!(home.is_empty());
        let w = s.workspace_mut(ws).unwrap();
        assert!(w.queries.is_empty() && w.tabs.is_empty() && w.diagram.is_none());
    }

    #[test]
    fn tree_refresh_only_for_workspaces_with_a_table_tab() {
        // Arrange
        let (mut redis, rws) = online_redis();
        let mut sql = state(&[]);
        let sws = sql.open_workspace(conn("s"));

        // Act
        let redis_cmd = redis
            .workspace_mut(rws)
            .and_then(|w| w.tree_if_supported())
            .and_then(|t| t.refresh_root());
        let sql_cmd = sql
            .workspace_mut(sws)
            .and_then(|w| w.tree_if_supported())
            .and_then(|t| t.refresh_root());

        // Assert
        assert!(redis_cmd.is_none());
        assert!(matches!(sql_cmd, Some(Command::LoadChildren { .. })));
    }
}
