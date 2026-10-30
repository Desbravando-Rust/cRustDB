#![allow(clippy::unwrap_used, clippy::expect_used)]
use crustdb_core::driver::ResultHeader;
use crustdb_core::driver::{TlsConfig, TlsMode};
use crustdb_core::model::{
    ColumnDetail, ColumnMeta, DriverKind, ObjectKind, ObjectRef, Product, TypeCategory,
};

use std::sync::atomic::{AtomicU64, Ordering};

use super::*;

static NEXT: AtomicU64 = AtomicU64::new(1);

pub(crate) fn conn(name: &str) -> ConnectionConfig {
    ConnectionConfig {
        id: ConnectionId(uuid::Uuid::from_u128(
            NEXT.fetch_add(1, Ordering::Relaxed).into(),
        )),
        name: name.into(),
        driver: DriverKind::Postgres,
        host: "localhost".into(),
        port: 5432,
        user: "u".into(),
        database: None,
        save_password: false,
        tls: TlsConfig {
            mode: TlsMode::VerifyFull,
            ca_path: None,
        },
        ssh: None,
        page_size: 300,
        editor_row_limit: 1000,
        safety_threshold: 1000,
        labels: vec![],
    }
}

pub(crate) fn state(names: &[&str]) -> AppState {
    AppState::new(LoadedConfig {
        connections: names.iter().map(|n| conn(n)).collect(),
        labels: vec![],
        read_only: false,
        warning: None,
    })
}

#[test]
fn apply_connected_marks_workspace_online() {
    // Arrange
    let mut s = state(&["a"]);
    let ws = s.open_workspace(s.config.connections[0].clone());
    assert_eq!(ws, WorkspaceId(1));
    let server = ServerVersion {
        product: Product::Postgres,
        major: 16,
        minor: 0,
        patch: 0,
        raw: "16.0".into(),
    };

    // Act
    s.apply(Event::Connected {
        ws,
        server: server.clone(),
    });

    // Assert
    assert_eq!(s.workspaces[0].status, Status::Online(server));
    assert!(!s.workspaces[0].form_open());
}

fn obj() -> ObjectRef {
    ObjectRef {
        database: "d".into(),
        schema: None,
        name: "t".into(),
        kind: ObjectKind::Table,
    }
}

fn detail(name: &str, pk: bool) -> ColumnDetail {
    ColumnDetail {
        meta: ColumnMeta {
            name: name.into(),
            type_name: "int4".into(),
            category: TypeCategory::Integer,
            nullable: Some(false),
            is_primary_key: pk,
        },
        default: None,
        comment: None,
    }
}

fn page(req: u64) -> Event {
    Event::Page {
        req: RequestId(req),
        header: ResultHeader {
            columns: vec![detail("x", false).meta],
        },
        rows: vec![],
        has_more: true,
        estimate: Some(5),
    }
}

#[test]
fn open_table_loads_structure_then_fetches_with_catalog_columns() {
    // Arrange
    let mut s = state(&["a"]);
    let ws = s.open_workspace(s.config.connections[0].clone());

    // Act & Assert
    let cmds = s.open_table(ws, obj(), RequestId(7));
    assert!(matches!(cmds[0], Command::OpenTable { tab: TabId(1), .. }));
    assert!(matches!(
        cmds[1],
        Command::LoadStructure {
            req: RequestId(7),
            ..
        }
    ));
    assert!(s.take_fetches(|| RequestId(8)).is_empty());
    s.apply(Event::Structure {
        req: RequestId(7),
        columns: vec![detail("id", true)],
        indexes: vec![],
        fks: vec![],
    });
    let f = s.take_fetches(|| RequestId(8));
    let [Command::FetchPage { req, query, .. }] = &f[..] else {
        panic!("expected one FetchPage");
    };
    assert_eq!(*req, RequestId(8));
    assert!(query.columns[0].is_primary_key);
    assert!(s.take_fetches(|| RequestId(9)).is_empty());
}

fn func() -> ObjectRef {
    ObjectRef {
        kind: ObjectKind::Function,
        ..obj()
    }
}

#[test]
fn open_object_function_emits_open_and_ddl() {
    // Arrange
    let mut s = state(&["a"]);
    let ws = s.open_workspace(s.config.connections[0].clone());

    // Act
    let cmds = s.open_table(ws, func(), RequestId(1));

    // Assert
    assert!(matches!(
        cmds[..],
        [
            Command::OpenTable { tab: TabId(1), .. },
            Command::LoadDdl {
                tab: TabId(1),
                req: RequestId(1)
            }
        ]
    ));
    let Some(Tab::Table(t)) = s.workspaces[0].tabs.first() else {
        panic!("tab");
    };
    assert_eq!(t.view, TableView::Ddl);
    assert!(s.take_fetches(|| RequestId(2)).is_empty());
}

#[test]
fn tab_ids_are_unique_across_workspaces_and_kinds() {
    // Arrange
    let mut s = state(&["a", "b"]);
    let a = s.open_workspace(s.config.connections[0].clone());
    let b = s.open_workspace(s.config.connections[1].clone());
    let mut ids = vec![];

    // Act
    for (ws, o) in [(a, obj()), (b, func()), (a, func()), (b, obj())] {
        for c in s.open_table(ws, o, RequestId(1)) {
            if let Command::OpenTable { tab, .. } = c {
                ids.push(tab.0);
            }
        }
    }

    // Assert
    assert_eq!(ids, [1, 2, 3, 4]);
}

#[test]
fn page_ignores_stale_req_and_keeps_catalog_columns() {
    // Arrange
    let mut s = state(&["a"]);
    let ws = s.open_workspace(s.config.connections[0].clone());
    s.open_table(ws, obj(), RequestId(7));
    s.apply(Event::Structure {
        req: RequestId(7),
        columns: vec![detail("id", true)],
        indexes: vec![],
        fks: vec![],
    });
    s.take_fetches(|| RequestId(8));

    // Act & Assert
    s.apply(page(6));
    let t = s.table_mut(TabId(1)).unwrap();
    assert!(t.loading.is_some() && t.estimate.is_none());
    s.apply(page(8));
    let t = s.table_mut(TabId(1)).unwrap();
    assert!(t.loading.is_none() && t.has_more && t.estimate == Some(5));
    assert_eq!(t.columns[0].name, "id");
    assert!(t.columns[0].is_primary_key);
}

#[test]
fn ddl_retry_reemits_load_ddl_and_event_fills_text() {
    // Arrange
    let mut s = state(&["a"]);
    let ws = s.open_workspace(s.config.connections[0].clone());
    s.open_table(ws, func(), RequestId(1));

    // Act & Assert
    s.apply(Event::Notice {
        tab: TabId(1),
        text: "boom".into(),
    });
    let t = s.table_mut(TabId(1)).unwrap();
    assert!(!t.is_loading());
    assert!(matches!(
        t.retry(RequestId(2)),
        Some(Command::LoadDdl {
            req: RequestId(2),
            ..
        })
    ));
    assert!(t.is_loading());
    s.apply(Event::Ddl {
        req: RequestId(2),
        text: "create".into(),
    });
    let t = s.table_mut(TabId(1)).unwrap();
    assert_eq!(t.ddl.as_deref(), Some("create"));
    assert!(!t.is_loading());
    t.notice = Some("stale".into());
    t.ddl_req = Some(RequestId(3));
    s.apply(Event::Ddl {
        req: RequestId(3),
        text: "create".into(),
    });
    assert!(s.table_mut(TabId(1)).unwrap().notice.is_none());
}

#[test]
fn notice_and_crash_release_loading() {
    // Arrange
    let mut s = state(&["a"]);
    let ws = s.open_workspace(s.config.connections[0].clone());
    s.open_table(ws, obj(), RequestId(1));

    // Act & Assert
    s.apply(Event::Notice {
        tab: TabId(1),
        text: "boom".into(),
    });
    let t = s.table_mut(TabId(1)).unwrap();
    assert!(t.structure_req.is_none());
    assert_eq!(t.notice.as_deref(), Some("boom"));
    t.fetch(RequestId(2));
    s.apply(Event::ActorCrashed { tab: TabId(1) });
    assert!(s.table_mut(TabId(1)).unwrap().loading.is_none());
}

#[test]
fn apply_connect_failed_keeps_form_open_with_error() {
    // Arrange
    let mut s = state(&["a"]);
    let ws = s.open_workspace(s.config.connections[0].clone());

    // Act
    s.apply(Event::ConnectFailed {
        ws,
        error: "refused".into(),
    });

    // Assert
    assert_eq!(s.workspaces[0].status, Status::Failed("refused".into()));
    assert!(s.workspaces[0].form_open());
}

#[test]
fn query_tab_opens_and_run_sends_evaluate_only() {
    // Arrange
    let mut s = state(&["a"]);
    let ws = s.open_workspace(s.config.connections[0].clone());
    let Some(Command::OpenEditor { tab, .. }) = s.open_query(ws, None) else {
        panic!("no OpenEditor");
    };
    s.workspace_mut(ws)
        .unwrap()
        .active_query_mut()
        .unwrap()
        .text = "select 1; select 2".into();

    // Act & Assert
    let Some(Command::Evaluate { statements, .. }) = s.run_query(tab, 3..3, false, RequestId(5))
    else {
        panic!("no Evaluate");
    };
    assert_eq!(statements, vec!["select 1"]);
    // Pending run: a repeat is refused.
    assert!(s.run_query(tab, 0..0, true, RequestId(6)).is_none());
    assert!(s.take_commands().is_empty());
    // Once evaluated (Run verdict -> Execute from the dialog stack), the tab can run again.
    s.apply(Event::Evaluated {
        req: RequestId(5),
        evaluation: crustdb_core::safety::Evaluation {
            verdict: crustdb_core::safety::Verdict::Run,
            flags: vec![],
        },
    });
    assert!(s.run_query(tab, 0..0, true, RequestId(7)).is_some());
}

#[test]
fn execute_marks_tab_running_and_routes_events_by_req() {
    // Arrange
    let mut s = state(&["a"]);
    let ws = s.open_workspace(s.config.connections[0].clone());
    let Some(Command::OpenEditor { tab, .. }) = s.open_query(ws, None) else {
        panic!("no OpenEditor");
    };
    s.query_mut(tab).unwrap().text = "select 1".into();
    s.run_query(tab, 0..0, true, RequestId(5)).unwrap();
    s.apply(Event::Evaluated {
        req: RequestId(5),
        evaluation: crustdb_core::safety::Evaluation {
            verdict: crustdb_core::safety::Verdict::Run,
            flags: vec![],
        },
    });

    // Act & Assert
    let cmd = s.take_commands().pop().unwrap();
    s.note_sent(&cmd);
    let Command::Execute { req, .. } = cmd else {
        panic!("no Execute");
    };
    assert_eq!(s.query_mut(tab).unwrap().running, Some(req));
    // A busy tab refuses a second run.
    assert!(s.run_query(tab, 0..0, true, RequestId(6)).is_none());
    s.apply(Event::Rows {
        req,
        rows: vec![],
        exhausted: true,
    });
    s.apply(Event::TxChanged {
        tab,
        state: crate::runtime::TxState::Open,
    });
    s.apply(Event::Notice {
        tab,
        text: "Connection lost — open transaction was rolled back by the server".into(),
    });
    let q = s.query_mut(tab).unwrap();
    assert_eq!(q.running, None);
    assert_eq!(q.tx, crate::runtime::TxState::Open);
    assert_eq!(q.pane.messages.len(), 1);
}

#[test]
fn run_selection_is_in_chars_not_bytes() {
    // Arrange
    let mut s = state(&["a"]);
    let ws = s.open_workspace(s.config.connections[0].clone());
    let Some(Command::OpenEditor { tab, .. }) = s.open_query(ws, None) else {
        panic!("no OpenEditor");
    };
    s.workspace_mut(ws)
        .unwrap()
        .active_query_mut()
        .unwrap()
        .text = "select 'ação🦀'; select 2".into();

    // Act
    // chars 0..16 = `select 'ação🦀';`
    let Some(Command::Evaluate { statements, .. }) = s.run_query(tab, 0..16, false, RequestId(1))
    else {
        panic!("no Evaluate");
    };

    // Assert
    assert_eq!(statements, vec!["select 'ação🦀'"]);
}

#[test]
fn delete_connection_requires_confirmation() {
    // Arrange
    let mut s = state(&["a", "b"]);
    let id = s.config.connections[0].id;

    // Act & Assert
    s.request_delete(id);
    assert_eq!(s.config.connections.len(), 2);
    assert!(matches!(
        s.dialogs.top(),
        Some(Dialog::DeleteConnection { id: i, .. }) if *i == id
    ));
    assert_eq!(s.confirm_delete(|_| Ok::<_, ()>(())), Ok(Some(id)));
    assert_eq!(s.config.connections.len(), 1);
    assert!(s.dialogs.top().is_none());
}

#[test]
fn read_only_config_refuses_delete() {
    // Arrange
    let mut s = state(&["a"]);
    s.config.read_only = true;

    // Act
    s.request_delete(s.config.connections[0].id);

    // Assert
    assert!(s.dialogs.top().is_none());
    assert_eq!(s.config.connections.len(), 1);
}

#[test]
fn failed_save_keeps_connection() {
    // Arrange
    let mut s = state(&["a"]);
    s.request_delete(s.config.connections[0].id);

    // Act & Assert
    assert_eq!(s.confirm_delete(|_| Err("io")), Err("io"));
    assert_eq!(s.config.connections.len(), 1);
}

#[test]
fn evaluated_routes_by_registered_request() {
    // Arrange
    use crustdb_core::safety::{Evaluation, Flag, FlagReason, Verdict};
    let ev = |verdict| Evaluation {
        verdict,
        flags: vec![Flag {
            index: 0,
            verdict,
            reason: FlagReason::NoWhere,
        }],
    };
    let mut s = state(&["a"]);
    let (ws, tab) = (WorkspaceId(5), TabId(6));
    s.register_evaluate(RequestId(1), ws, tab);

    // Act & Assert
    s.apply(Event::Evaluated {
        req: RequestId(1),
        evaluation: ev(Verdict::Confirm),
    });
    assert!(matches!(
        s.dialogs.top(),
        Some(Dialog::Safety { ws: w, tab: t, req: RequestId(1), .. }) if *w == ws && *t == tab
    ));
    assert!(s.take_commands().is_empty());

    let mut s = state(&["a"]);
    s.apply(Event::Evaluated {
        req: RequestId(9),
        evaluation: ev(Verdict::Confirm),
    });
    assert!(s.dialogs.top().is_none() && s.take_commands().is_empty());

    s.register_evaluate(RequestId(2), ws, tab);
    s.apply(Event::Evaluated {
        req: RequestId(2),
        evaluation: Evaluation {
            verdict: Verdict::Run,
            flags: vec![],
        },
    });
    let cmds = s.take_commands();
    assert!(matches!(
        cmds[..],
        [Command::Execute {
            evaluation: RequestId(2),
            ..
        }]
    ));
    // Single use: a repeated answer is ignored.
    s.apply(Event::Evaluated {
        req: RequestId(2),
        evaluation: Evaluation {
            verdict: Verdict::Run,
            flags: vec![],
        },
    });
    assert!(s.take_commands().is_empty());
}

#[test]
fn live_workspace_ignores_failed_and_returns_latest() {
    // Arrange
    let mut s = state(&["a", "b"]);
    let (a, b) = (s.config.connections[0].id, s.config.connections[1].id);

    // Act & Assert
    assert_eq!(s.live_workspace(a), None);
    let ws = s.open_workspace(s.config.connections[0].clone());
    assert_eq!(s.live_workspace(a), Some(WorkspaceId(1)));
    s.workspaces[0].status = Status::Failed("refused".into());
    assert_eq!(s.live_workspace(a), None);
    s.workspaces[0].status = Status::Connecting;
    assert_eq!(s.live_workspace(a), Some(ws));
    assert_eq!(s.live_workspace(b), None);
}

#[test]
fn ctrl_f_toggles_find_on_query_tabs_and_filter_on_table_tabs() {
    // Arrange
    let mut s = state(&["a"]);
    let ws = s.open_workspace(s.config.connections[0].clone());
    s.open_table(ws, obj(), RequestId(1));

    // Act & Assert
    let w = s.workspace_mut(ws).unwrap();
    w.toggle_find_or_filter();
    assert!(w.active_table_mut().unwrap().filter_open);
    s.open_query(ws, None);
    let w = s.workspace_mut(ws).unwrap();
    w.toggle_find_or_filter();
    let q = w.active_query_mut().unwrap();
    assert!(q.find.open && q.find.focus);
    w.toggle_find_or_filter();
    assert!(!w.active_query_mut().unwrap().find.open);
}
