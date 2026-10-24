#![allow(clippy::unwrap_used, clippy::expect_used)]
use crustdb_core::model::{ObjectKind, Product};

use super::tests::state;
use super::*;
use crate::runtime::TxState;

#[test]
fn open_workspace_reuses_failed_workspace_of_same_connection() {
    // Arrange
    let mut s = state(&["a", "b"]);
    let (a, b) = (
        s.config.connections[0].clone(),
        s.config.connections[1].clone(),
    );
    let failed_a = s.open_workspace(a.clone());
    s.apply(Event::ConnectFailed {
        ws: failed_a,
        error: "no password".into(),
    });
    let failed_b = s.open_workspace(b);
    s.apply(Event::ConnectFailed {
        ws: failed_b,
        error: "refused".into(),
    });

    // Act
    let retry_a = s.open_workspace(a.clone());

    // Assert
    let of_a: Vec<_> = s
        .workspaces
        .iter()
        .filter(|w| w.config.id == a.id)
        .collect();
    assert_eq!(of_a.len(), 1);
    assert_eq!(retry_a, failed_a);
    assert_eq!(of_a[0].status, Status::Connecting);
    let b_ws = s.workspaces.iter().find(|w| w.id == failed_b).unwrap();
    assert!(matches!(b_ws.status, Status::Failed(_)));
    assert_eq!(s.active, Some(failed_a));
}

#[test]
fn reconnect_after_online_failure_keeps_tabs_and_reopens_their_actors() {
    // Arrange
    let mut s = state(&["a"]);
    let a = s.config.connections[0].clone();
    let ws = s.open_workspace(a.clone());
    s.apply(Event::Connected {
        ws,
        server: ServerVersion {
            product: Product::Postgres,
            major: 16,
            minor: 0,
            patch: 0,
            raw: "16.0".into(),
        },
    });
    let object = ObjectRef {
        database: "d".into(),
        schema: None,
        name: "t".into(),
        kind: ObjectKind::Table,
    };
    s.open_table(ws, object.clone(), RequestId(1));
    let Some(Command::OpenEditor { tab: query, .. }) = s.open_query(ws, None) else {
        panic!("expected OpenEditor");
    };
    let q = s.query_mut(query).unwrap();
    q.text = "select 1".into();
    q.dirty = true;
    q.tx = TxState::Open;
    q.running = Some(RequestId(2));
    q.bg = true;
    s.apply(Event::ConnectFailed {
        ws,
        error: "ssh tunnel closed".into(),
    });

    // Act
    let retry = s.open_workspace(a);

    // Assert
    assert_eq!(retry, ws);
    assert_eq!(s.workspaces.len(), 1);
    let w = &s.workspaces[0];
    assert_eq!(w.status, Status::Connecting);
    assert_eq!(w.tabs.len(), 1);
    assert_eq!(w.active_tab, Some(query));
    let q = &w.queries[0];
    assert_eq!((q.text.as_str(), q.dirty), ("select 1", true));
    assert_eq!((q.tx, q.running, q.bg), (TxState::Idle, None, false));
    assert!(q.can_run());
    let cmds = s.take_commands();
    assert!(cmds.iter().any(
        |c| matches!(c, Command::OpenTable { ws: w, object: o, .. } if *w == ws && *o == object)
    ));
    assert!(
        cmds.iter().any(
            |c| matches!(c, Command::OpenEditor { ws: w, tab, .. } if *w == ws && *tab == query)
        )
    );
}
