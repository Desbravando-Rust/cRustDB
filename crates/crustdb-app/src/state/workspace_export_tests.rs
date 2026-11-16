#![allow(clippy::unwrap_used, clippy::expect_used)]
use crustdb_core::model::{Cell, ObjectKind, ObjectRef};

use crate::runtime::{Event, RequestId};
use crate::state::AppState;
use crate::state::workspace::tests::state;

/// A table tab (id 1) exporting under req 7 and a query tab (id 2) whose pane is on req 9.
fn setup() -> AppState {
    let mut s = state(&["a"]);
    let ws = s.open_workspace(s.config.connections[0].clone());
    let object = ObjectRef {
        database: "d".into(),
        schema: None,
        name: "t".into(),
        kind: ObjectKind::Table,
    };
    s.open_table(ws, object, RequestId(1));
    s.open_query(ws, None);
    let w = s.workspace_mut(ws).unwrap();
    let Some(t) = w.tabs.first_mut() else {
        panic!("no tab");
    };
    t.rows = vec![vec![Cell::Null].into()];
    t.export(std::path::PathBuf::from("/tmp/t.csv"), RequestId(7));
    w.queries[0].pane.req = Some(RequestId(9));
    s
}

fn table_notice(s: &mut AppState) -> (Option<String>, bool) {
    let Some(t) = s.workspaces[0].tabs.first() else {
        panic!("no tab");
    };
    (t.notice.clone(), t.export_req.is_some())
}

#[test]
fn export_done_reaches_the_table_tab_only() {
    // Arrange
    let mut s = setup();

    // Act
    s.apply(Event::ExportDone {
        req: RequestId(7),
        result: Ok(5),
    });

    // Assert
    assert_eq!(
        table_notice(&mut s),
        (Some("exported 5 rows".into()), false)
    );
    assert!(s.workspaces[0].queries[0].pane.messages.is_empty());
}

#[test]
fn export_done_reaches_the_query_tab_only() {
    // Arrange
    let mut s = setup();

    // Act
    s.apply(Event::ExportDone {
        req: RequestId(9),
        result: Ok(3),
    });

    // Assert
    assert_eq!(s.workspaces[0].queries[0].pane.messages.len(), 1);
    assert_eq!(table_notice(&mut s), (None, true));
}

#[test]
fn export_done_error_is_reported_on_the_table_tab() {
    // Arrange
    let mut s = setup();

    // Act
    s.apply(Event::ExportDone {
        req: RequestId(7),
        result: Err("disk full".into()),
    });

    // Assert
    assert_eq!(
        table_notice(&mut s),
        (Some("export failed: disk full".into()), false)
    );
}

#[test]
fn export_done_for_unknown_req_changes_nothing() {
    // Arrange
    let mut s = setup();

    // Act
    s.apply(Event::ExportDone {
        req: RequestId(99),
        result: Ok(1),
    });

    // Assert
    assert_eq!(table_notice(&mut s), (None, true));
    assert!(s.workspaces[0].queries[0].pane.messages.is_empty());
}
