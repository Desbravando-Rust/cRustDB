#![allow(clippy::unwrap_used, clippy::expect_used)]
use crate::runtime::{Command, Event, RequestId, TxOp, TxState};
use crate::state::workspace::tests::state;
use crate::state::{Dialog, Tab};
use crustdb_core::model::{ObjectKind, ObjectRef};

fn table_ref() -> ObjectRef {
    ObjectRef {
        database: "d".into(),
        schema: None,
        name: "t".into(),
        kind: ObjectKind::Table,
    }
}

/// Workspace with one query tab holding an open transaction.
fn with_tx() -> (
    crate::state::workspace::AppState,
    crate::runtime::WorkspaceId,
    crate::runtime::TabId,
) {
    let mut s = state(&["a"]);
    let ws = s.open_workspace(s.config.connections[0].clone());
    s.open_query(ws, None);
    let tab = s.workspaces[0].queries[0].id;
    s.workspaces[0].queries[0].tx = TxState::Open;
    (s, ws, tab)
}

#[test]
fn request_disconnect_without_tx_or_pending_disconnects_directly() {
    // Arrange
    let mut s = state(&["a"]);
    let ws = s.open_workspace(s.config.connections[0].clone());

    // Act
    let cmds = s.request_disconnect(ws);

    // Assert
    assert!(matches!(cmds.as_slice(), [Command::Disconnect { ws: w }] if *w == ws));
    assert!(s.workspaces.is_empty());
}

#[test]
fn request_disconnect_with_open_tx_asks() {
    // Arrange
    let (mut s, ws, _) = with_tx();

    // Act
    let cmds = s.request_disconnect(ws);

    // Assert
    assert!(cmds.is_empty());
    assert_eq!(s.workspaces.len(), 1);
    assert_eq!(
        s.dialogs.top(),
        Some(&Dialog::Disconnect {
            ws,
            has_tx: true,
            pending_rows: 0
        })
    );
}

#[test]
fn request_disconnect_with_pending_edit_asks() {
    // Arrange
    let mut s = state(&["a"]);
    let ws = s.open_workspace(s.config.connections[0].clone());
    let object = ObjectRef {
        database: "d".into(),
        schema: None,
        name: "t".into(),
        kind: ObjectKind::Table,
    };
    s.open_table(ws, object, RequestId(1));
    let Some(Tab::Table(t)) = s.workspaces[0].tabs.first_mut() else {
        panic!("expected table tab");
    };
    t.pending.push((0, Some("x".into())));

    // Act
    let cmds = s.request_disconnect(ws);

    // Assert
    assert!(cmds.is_empty());
    assert_eq!(
        s.dialogs.top(),
        Some(&Dialog::Disconnect {
            ws,
            has_tx: false,
            pending_rows: 1
        })
    );
}

fn resolve_waits_for_idle(op: TxOp) {
    // Arrange
    let (mut s, ws, tab) = with_tx();
    s.request_disconnect(ws);

    // Act
    let cmds = s.resolve_disconnect(op);

    // Assert
    assert!(matches!(
        cmds.as_slice(),
        [Command::Tx { tab: t, op: o }] if *t == tab && *o == op
    ));
    assert_eq!(s.workspaces.len(), 1);
    assert!(s.take_commands().is_empty());

    // Act
    s.apply(Event::TxChanged {
        tab,
        state: TxState::Idle,
    });

    // Assert
    assert!(s.workspaces.is_empty());
    assert!(matches!(s.take_commands().as_slice(), [Command::Disconnect { ws: w }] if *w == ws));
}

#[test]
fn resolve_disconnect_commit_waits_for_idle() {
    resolve_waits_for_idle(TxOp::Commit);
}

#[test]
fn resolve_disconnect_rollback_waits_for_idle() {
    resolve_waits_for_idle(TxOp::Rollback);
}

/// Commit sent through `note_sent`, then a failure Notice and the late `TxChanged{Idle}`.
fn failed_resolve(text: &str) {
    // Arrange
    let (mut s, ws, tab) = with_tx();
    s.request_disconnect(ws);
    for c in s.resolve_disconnect(TxOp::Commit) {
        s.note_sent(&c);
    }

    // Act
    s.apply(Event::Notice {
        tab,
        text: text.into(),
    });
    s.apply(Event::TxChanged {
        tab,
        state: TxState::Idle,
    });

    // Assert
    assert_eq!(s.workspaces.len(), 1);
    assert_eq!(s.workspaces[0].queries.len(), 1);
    assert_eq!(s.workspaces[0].queries[0].pane.messages.len(), 1);
    assert!(s.take_commands().is_empty());
}

#[test]
fn failed_commit_through_note_sent_shows_error_and_stays_connected() {
    failed_resolve("commit failed");
}

#[test]
fn no_actor_commit_through_note_sent_shows_error_and_stays_connected() {
    failed_resolve("no actor for this command");
}

#[test]
fn actor_crash_while_resolving_cancels_disconnect() {
    // Arrange
    let (mut s, ws, tab) = with_tx();
    s.request_disconnect(ws);
    s.resolve_disconnect(TxOp::Commit);

    // Act
    s.apply(Event::ActorCrashed { tab });
    s.apply(Event::TxChanged {
        tab,
        state: TxState::Idle,
    });

    // Assert
    assert_eq!(s.workspaces.len(), 1);
    assert!(s.take_commands().is_empty());
}

#[test]
fn actor_crash_resets_tx_so_disconnect_is_direct() {
    // Arrange
    let (mut s, ws, tab) = with_tx();

    // Act
    s.apply(Event::ActorCrashed { tab });
    let cmds = s.request_disconnect(ws);

    // Assert
    assert!(matches!(cmds.as_slice(), [Command::Disconnect { ws: w }] if *w == ws));
}

#[test]
fn failed_commit_of_one_tab_keeps_workspace_connected_and_other_tab_alive() {
    // Arrange
    let (mut s, ws, a) = with_tx();
    s.open_query(ws, None);
    let b = s.workspaces[0].queries[1].id;
    s.workspaces[0].queries[1].tx = TxState::Open;
    s.request_disconnect(ws);
    let cmds = s.resolve_disconnect(TxOp::Commit);

    // Act
    s.apply(Event::Notice {
        tab: a,
        text: "commit failed".into(),
    });
    for tab in [a, b] {
        s.apply(Event::TxChanged {
            tab,
            state: TxState::Idle,
        });
    }

    // Assert
    assert!(cmds.iter().all(|c| matches!(c, Command::Tx { .. })));
    assert_eq!(s.workspaces.len(), 1);
    assert_eq!(s.workspaces[0].queries.len(), 2);
    assert!(s.take_commands().is_empty());
}

#[test]
fn new_pending_edit_during_resolve_reopens_dialog() {
    // Arrange
    let (mut s, ws, tab) = with_tx();
    s.request_disconnect(ws);
    s.resolve_disconnect(TxOp::Commit);
    s.open_table(ws, table_ref(), RequestId(1));
    let Some(Tab::Table(t)) = s.workspaces[0].tabs.first_mut() else {
        panic!("expected table tab");
    };
    t.pending.push((0, Some("x".into())));

    // Act
    s.apply(Event::TxChanged {
        tab,
        state: TxState::Idle,
    });

    // Assert
    assert_eq!(s.workspaces.len(), 1);
    assert!(s.take_commands().is_empty());
    assert_eq!(
        s.dialogs.top(),
        Some(&Dialog::Disconnect {
            ws,
            has_tx: false,
            pending_rows: 1
        })
    );
}

#[test]
fn sibling_query_running_during_resolve_reopens_dialog() {
    // Arrange
    let (mut s, ws, a) = with_tx();
    s.open_query(ws, None);
    let b = s.workspaces[0].queries[1].id;
    s.request_disconnect(ws);
    s.resolve_disconnect(TxOp::Commit);
    s.register_evaluate(RequestId(9), ws, b);

    // Act
    s.apply(Event::TxChanged {
        tab: a,
        state: TxState::Idle,
    });

    // Assert
    assert_eq!(s.workspaces.len(), 1);
    assert!(s.take_commands().is_empty());
    assert_eq!(
        s.dialogs.top(),
        Some(&Dialog::Disconnect {
            ws,
            has_tx: true,
            pending_rows: 0
        })
    );
}

#[test]
fn resolve_disconnect_rejects_savepoint() {
    // Arrange
    let (mut s, ws, _) = with_tx();
    s.request_disconnect(ws);

    // Act
    let rejected = s.resolve_disconnect(TxOp::Savepoint);

    // Assert
    assert!(rejected.is_empty());
    assert!(s.dialogs.top().is_some());
}

#[test]
fn cancel_disconnect_keeps_tx_and_pending_edit() {
    // Arrange
    let (mut s, ws, _) = with_tx();
    s.open_table(ws, table_ref(), RequestId(1));
    let Some(Tab::Table(t)) = s.workspaces[0].tabs.first_mut() else {
        panic!("expected table tab");
    };
    t.pending.push((0, Some("x".into())));
    s.request_disconnect(ws);

    // Act
    let cancelled = s.dialogs.cancel();

    // Assert
    assert!(cancelled.is_none());
    assert!(s.dialogs.top().is_none());
    assert_eq!(s.workspaces.len(), 1);
    assert_eq!(s.workspaces[0].queries[0].tx, TxState::Open);
    let Some(Tab::Table(t)) = s.workspaces[0].tabs.first() else {
        panic!("expected table tab");
    };
    assert!(!t.pending.is_empty());
}

#[test]
fn close_workspace_drops_disconnect_dialog() {
    // Arrange
    let (mut s, ws, _) = with_tx();
    s.request_disconnect(ws);

    // Act
    s.close_workspace(ws);

    // Assert
    assert!(s.dialogs.top().is_none());
}

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
