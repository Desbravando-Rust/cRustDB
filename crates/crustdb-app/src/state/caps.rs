//! Capability gate: which screens and actions a workspace's data model supports.
use crustdb_core::model::Capabilities;

use super::WorkspaceState;

impl WorkspaceState {
    /// The single source for every "is this action available" decision.
    pub fn capabilities(&self) -> Capabilities {
        self.config.driver.data_model().capabilities()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use crustdb_core::model::{DriverKind, ObjectKind, ObjectRef, Product, ServerVersion};

    use crate::runtime::{Event, RequestId};
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
        assert!(!w.capabilities().table_export_csv);
    }
}
