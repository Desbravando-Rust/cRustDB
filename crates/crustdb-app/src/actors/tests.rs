#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::sync::atomic::Ordering;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use crustdb_core::driver::{SessionPurpose, TlsConfig, TlsMode};
use crustdb_core::model::{
    Cell, ColumnMeta, ConnectionId, DriverKind, ObjectKind, ObjectRef, TypeCategory,
};
use crustdb_core::query::{Filter, Operator, TableQuery};
use crustdb_testkit::fake_driver::{FakeDriver, FakeScript};

use super::Actors;
use super::workspace::backoff;
use crate::runtime::{
    Command, DriverRegistry, Emitter, Event, RequestId, ResolvedSecrets, TabId, WorkspaceId,
};
use crate::storage::config::ConnectionConfig;

pub(super) const WS: WorkspaceId = WorkspaceId(1);
pub(super) const TAB: TabId = TabId(1);

pub(super) struct H {
    pub(super) actors: Actors,
    pub(super) rx: mpsc::Receiver<Event>,
    pub(super) drv: Arc<FakeDriver>,
}

pub(super) fn harness(script: FakeScript) -> H {
    harness_with(FakeDriver::new(script))
}

pub(super) fn harness_with(drv: Arc<FakeDriver>) -> H {
    let reg = DriverRegistry::default().with(drv.clone());
    let (tx, rx) = mpsc::channel();
    let out = Emitter {
        tx,
        repaint: Arc::new(|| {}),
    };
    H {
        actors: Actors::new(reg, out),
        rx,
        drv,
    }
}

pub(super) fn config(database: Option<&str>) -> ConnectionConfig {
    ConnectionConfig {
        id: ConnectionId(uuid::Uuid::nil()),
        name: "t".into(),
        driver: DriverKind::Postgres,
        host: "h".into(),
        port: 5432,
        user: "u".into(),
        database: database.map(String::from),
        save_password: false,
        tls: TlsConfig {
            mode: TlsMode::Disable,
            ca_path: None,
            client_cert_path: None,
            client_key_path: None,
        },
        ssh: None,
        page_size: 300,
        editor_row_limit: 1000,
        safety_threshold: 1000,
        label: None,
        key_separator: None,
    }
}

pub(super) fn col(name: &str) -> ColumnMeta {
    ColumnMeta {
        name: name.into(),
        type_name: "int4".into(),
        category: TypeCategory::Integer,
        nullable: Some(false),
        is_primary_key: true,
    }
}

fn object() -> ObjectRef {
    ObjectRef {
        database: "d".into(),
        schema: Some("public".into()),
        name: "t".into(),
        kind: ObjectKind::Table,
    }
}

pub(super) fn query_for_export() -> TableQuery {
    query(300, vec![])
}

pub(super) fn query(limit: u64, filters: Vec<Filter>) -> TableQuery {
    TableQuery {
        object: object(),
        columns: vec![col("id")],
        filters,
        sorts: vec![],
        limit: Some(limit),
        offset: 0,
        after: None,
    }
}

pub(super) fn rows(n: usize) -> Vec<crustdb_core::model::Row> {
    (0..n)
        .map(|i| {
            vec![Cell::Text {
                value: i.to_string().into(),
                truncated: false,
            }]
            .into()
        })
        .collect()
}

pub(super) async fn next(rx: &mpsc::Receiver<Event>) -> Event {
    for _ in 0..10_000 {
        if let Ok(e) = rx.try_recv() {
            return e;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("no event");
}

pub(super) fn drain(rx: &mpsc::Receiver<Event>) -> Vec<Event> {
    rx.try_iter().collect()
}

pub(super) async fn connect(h: &mut H, database: Option<&str>) {
    assert!(
        h.actors
            .dispatch(Command::Connect {
                ws: WS,
                config: config(database),
                secrets: ResolvedSecrets::default(),
            })
            .is_none()
    );
    assert!(matches!(next(&h.rx).await, Event::Connected { .. }));
}

pub(super) async fn open_table(h: &mut H) {
    assert!(
        h.actors
            .dispatch(Command::OpenTable {
                ws: WS,
                tab: TAB,
                object: object(),
            })
            .is_none()
    );
}

pub(super) fn fetch(h: &mut H, req: u64, q: TableQuery) {
    assert!(
        h.actors
            .dispatch(Command::FetchPage {
                tab: TAB,
                req: RequestId(req),
                query: q,
            })
            .is_none()
    );
}

pub(super) async fn page(h: &mut H) -> (RequestId, usize, bool, Option<u64>) {
    match next(&h.rx).await {
        Event::Page {
            req,
            rows,
            has_more,
            estimate,
            ..
        } => (req, rows.len(), has_more, estimate),
        e => panic!("expected Page, got {e:?}"),
    }
}

pub(super) fn connects(h: &H) -> Vec<(SessionPurpose, Option<String>)> {
    let c = h.drv.log.connects.lock().unwrap();
    c.iter().map(|(p, d, _)| (*p, d.clone())).collect()
}

#[tokio::test]
async fn connect_opens_metadata_session_with_metadata_purpose() {
    // Arrange
    let mut h = harness(FakeScript::default());

    // Act
    connect(&mut h, None).await;

    // Assert
    assert_eq!(connects(&h), [(SessionPurpose::Metadata, None)]);
}

#[tokio::test]
async fn pg_second_database_opens_second_metadata_session() {
    // Arrange
    let mut h = harness(FakeScript::default());
    connect(&mut h, Some("a")).await;

    // Act & Assert
    for _ in 0..2 {
        h.actors.dispatch(Command::LoadChildren {
            ws: WS,
            req: RequestId(1),
            database: "b".into(),
            schema: Some("public".into()),
            kind: Some(ObjectKind::Table),
        });
        assert!(matches!(next(&h.rx).await, Event::Children { .. }));
    }
    assert_eq!(
        connects(&h),
        [
            (SessionPurpose::Metadata, Some("a".into())),
            (SessionPurpose::Metadata, Some("b".into()))
        ]
    );
}

#[tokio::test]
async fn page_has_more_when_rows_reach_limit_including_lookahead() {
    // Arrange
    let mut h = harness(FakeScript {
        rows: rows(3),
        ..FakeScript::default()
    });
    connect(&mut h, None).await;
    open_table(&mut h).await;

    // Act & Assert
    fetch(&mut h, 1, query(2, vec![]));
    assert_eq!(page(&mut h).await, (RequestId(1), 2, true, None));
    fetch(&mut h, 2, query(4, vec![]));
    assert_eq!(page(&mut h).await, (RequestId(2), 3, false, None));
    assert!(connects(&h).iter().any(|(p, _)| *p == SessionPurpose::Data));
}

#[tokio::test]
async fn page_has_no_more_for_zero_limit() {
    // Arrange
    let mut h = harness(FakeScript {
        rows: rows(3),
        ..FakeScript::default()
    });
    connect(&mut h, None).await;
    open_table(&mut h).await;

    // Act
    fetch(&mut h, 1, query(0, vec![]));
    let (_, n, has_more, _) = page(&mut h).await;

    // Assert
    assert_eq!((n, has_more), (0, false));
}

#[tokio::test]
async fn page_attaches_estimate_only_without_filters() {
    // Arrange
    let mut h = harness(FakeScript {
        rows: rows(1),
        estimate: Some(1234),
        ..FakeScript::default()
    });
    connect(&mut h, None).await;
    open_table(&mut h).await;

    // Act & Assert
    fetch(&mut h, 1, query(10, vec![]));
    assert_eq!(page(&mut h).await.3, Some(1234));
    let f = Filter {
        column: "id".into(),
        op: Operator::Eq,
        values: vec!["1".into()],
    };
    fetch(&mut h, 2, query(10, vec![f]));
    assert_eq!(page(&mut h).await.3, None);
}

#[tokio::test(start_paused = true)]
async fn stale_request_id_ignored() {
    // Arrange
    let mut h = harness(FakeScript {
        rows: rows(1),
        delay: Duration::from_secs(10),
        ..FakeScript::default()
    });
    connect(&mut h, None).await;
    open_table(&mut h).await;

    // Act
    fetch(&mut h, 1, query(10, vec![]));
    tokio::time::sleep(Duration::from_secs(1)).await;
    fetch(&mut h, 2, query(10, vec![]));
    tokio::time::sleep(Duration::from_secs(30)).await;

    // Assert
    let pages: Vec<_> = drain(&h.rx)
        .into_iter()
        .filter_map(|e| match e {
            Event::Page { req, .. } => Some(req),
            Event::Notice { .. } => panic!("stale request must be silent"),
            _ => None,
        })
        .collect();
    assert_eq!(pages, [RequestId(2)]);
    assert_eq!(h.drv.log.cancels.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn cancel_tab_cancels_running_page() {
    // Arrange
    let mut h = harness(FakeScript {
        rows: rows(1),
        delay: Duration::from_secs(10),
        ..FakeScript::default()
    });
    connect(&mut h, None).await;
    open_table(&mut h).await;

    // Act
    fetch(&mut h, 1, query(10, vec![]));
    tokio::time::sleep(Duration::from_secs(1)).await;
    h.actors.dispatch(Command::CancelTab { tab: TAB });
    tokio::time::sleep(Duration::from_secs(30)).await;

    // Assert
    let evs = drain(&h.rx);
    assert!(evs.iter().all(|e| !matches!(e, Event::Page { .. })));
    assert!(
        evs.iter()
            .any(|e| matches!(e, Event::Notice { tab: TAB, text } if text.contains("cancel")))
    );
    assert_eq!(h.drv.log.cancels.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn ping_every_60s_emits_latency() {
    // Arrange
    let mut h = harness(FakeScript {
        rows: rows(1),
        ..FakeScript::default()
    });
    connect(&mut h, None).await;
    open_table(&mut h).await;
    fetch(&mut h, 1, query(10, vec![]));
    page(&mut h).await;

    // Act & Assert
    tokio::time::sleep(Duration::from_secs(59)).await;
    assert!(drain(&h.rx).is_empty());
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(matches!(next(&h.rx).await, Event::Ping { ws: WS, .. }));
    // Only the Metadata session is pinged, never the Data one.
    assert_eq!(h.drv.log.pings.load(Ordering::SeqCst), 1);
}

#[test]
fn backoff_doubles_up_to_30s() {
    // Act
    let secs: Vec<_> = (0..7).map(|n| backoff(n).as_secs()).collect();

    // Assert
    assert_eq!(secs, [1, 2, 4, 8, 16, 30, 30]);
}

#[tokio::test(start_paused = true)]
async fn metadata_reconnects_with_backoff() {
    // Arrange
    let mut h = harness(FakeScript::default());
    connect(&mut h, None).await;
    h.drv.log.fail_pings.store(true, Ordering::SeqCst);
    h.drv.log.fail_connects.store(4, Ordering::SeqCst);

    // Act
    tokio::time::sleep(Duration::from_secs(100)).await;

    // Assert
    let t: Vec<_> = h
        .drv
        .log
        .connects
        .lock()
        .unwrap()
        .iter()
        .map(|c| c.2)
        .collect();
    assert_eq!(t.len(), 6, "initial + 4 failures + 1 success");
    let gaps: Vec<_> = t.windows(2).map(|w| (w[1] - w[0]).as_secs()).collect();
    // ping at 60 s fails, then 1, 2, 4, 8 and 16 s waits
    assert_eq!(gaps, [61, 2, 4, 8, 16]);
}

#[tokio::test]
async fn actor_panic_emits_actor_crashed_and_restarts() {
    // Arrange
    let mut h = harness(FakeScript {
        rows: rows(1),
        ..FakeScript::default()
    });
    connect(&mut h, None).await;
    open_table(&mut h).await;
    h.drv.log.panic_next.store(true, Ordering::SeqCst);

    // Act & Assert
    fetch(&mut h, 1, query(10, vec![]));
    assert!(matches!(
        next(&h.rx).await,
        Event::ActorCrashed { tab: TAB }
    ));
    fetch(&mut h, 2, query(10, vec![]));
    assert_eq!(page(&mut h).await.0, RequestId(2));
}

#[tokio::test(start_paused = true)]
async fn reconnecting_same_workspace_stops_old_actor() {
    // Arrange
    let mut h = harness(FakeScript::default());
    connect(&mut h, None).await;

    // Act
    connect(&mut h, None).await;
    tokio::time::sleep(Duration::from_secs(61)).await;

    // Assert
    let pings = drain(&h.rx)
        .into_iter()
        .filter(|e| matches!(e, Event::Ping { .. }))
        .count();
    assert_eq!(pings, 1);
}

#[tokio::test(start_paused = true)]
async fn ping_failure_emits_reconnecting_then_connected() {
    // Arrange
    let mut h = harness(FakeScript::default());
    connect(&mut h, None).await;
    h.drv.log.fail_pings.store(true, Ordering::SeqCst);

    // Act
    tokio::time::sleep(Duration::from_secs(61)).await;

    // Assert
    assert!(matches!(next(&h.rx).await, Event::Reconnecting { ws: WS }));
    assert!(matches!(next(&h.rx).await, Event::Connected { ws: WS, .. }));
}

#[tokio::test(start_paused = true)]
async fn workspace_actor_panic_resets_connection_state() {
    // Arrange
    let mut h = harness(FakeScript::default());
    connect(&mut h, None).await;
    h.drv.log.panic_ping.store(true, Ordering::SeqCst);

    // Act & Assert
    tokio::time::sleep(Duration::from_secs(61)).await;
    let evs = drain(&h.rx);
    assert!(
        evs.iter()
            .any(|e| matches!(e, Event::ActorCrashed { tab: TabId(0) }))
    );
    assert!(
        evs.iter().any(
            |e| matches!(e, Event::ConnectFailed { ws: WS, error } if error.contains("crashed"))
        )
    );
    open_table(&mut h).await;
    fetch(&mut h, 1, query(10, vec![]));
    assert!(matches!(next(&h.rx).await, Event::Notice { .. }));
}

#[tokio::test]
async fn load_children_failure_still_answers_with_empty_children() {
    // Arrange
    let mut h = harness(FakeScript::default());
    connect(&mut h, Some("a")).await;
    h.drv.log.fail_connects.store(1, Ordering::SeqCst);

    // Act
    h.actors.dispatch(Command::LoadChildren {
        ws: WS,
        req: RequestId(5),
        database: "b".into(),
        schema: Some("public".into()),
        kind: Some(ObjectKind::Table),
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Assert
    let ev = drain(&h.rx);
    assert!(ev.iter().any(|e| matches!(e, Event::Notice { .. })));
    assert!(
        ev.iter()
            .any(|e| matches!(e, Event::Children { req: RequestId(5), items } if items.is_empty()))
    );
}

#[tokio::test]
async fn load_columns_uses_the_metadata_session_only() {
    // Arrange
    let mut h = harness(FakeScript::default());
    connect(&mut h, Some("a")).await;
    let mut object = object();
    object.database = "a".into();

    // Act
    h.actors.dispatch(Command::LoadColumns {
        ws: WS,
        req: RequestId(7),
        object,
    });

    // Assert
    let Event::Columns { req, columns } = next(&h.rx).await else {
        panic!("expected Columns");
    };
    assert_eq!(req, RequestId(7));
    assert_eq!(columns, Some(vec![]));
    assert_eq!(connects(&h), [(SessionPurpose::Metadata, Some("a".into()))]);
}

#[tokio::test]
async fn load_columns_failure_answers_none() {
    // Arrange
    let mut h = harness(FakeScript::default());
    connect(&mut h, Some("a")).await;
    h.drv.log.fail_connects.store(1, Ordering::SeqCst);
    let mut object = object();
    object.database = "other".into();

    // Act
    h.actors.dispatch(Command::LoadColumns {
        ws: WS,
        req: RequestId(8),
        object,
    });

    // Assert
    assert!(matches!(
        next(&h.rx).await,
        Event::Columns {
            req: RequestId(8),
            columns: None
        }
    ));
}

#[tokio::test]
async fn sqlite_editor_opens_configured_file() {
    // Arrange
    let script = FakeScript {
        columns: vec![col("id")],
        rows: rows(1),
        ..FakeScript::default()
    };
    let mut h = harness_with(FakeDriver::with_kind(script, DriverKind::Sqlite));
    let mut config = config(Some("/tmp/a.db"));
    config.driver = DriverKind::Sqlite;
    let secrets = ResolvedSecrets::default();
    let (ws, tab) = (WS, TAB);
    h.actors.dispatch(Command::Connect {
        ws,
        config,
        secrets,
    });
    assert!(matches!(next(&h.rx).await, Event::Connected { .. }));

    // Act
    let database = Some("main".to_string());
    h.actors.dispatch(Command::OpenEditor { ws, tab, database });
    let statements = vec!["SELECT 1".to_string()];
    h.actors.dispatch(Command::Evaluate {
        tab,
        req: RequestId(1),
        statements,
    });
    h.actors.dispatch(Command::Execute {
        tab,
        req: RequestId(2),
        evaluation: RequestId(1),
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Assert
    let file = Some("/tmp/a.db".to_string());
    assert!(connects(&h).contains(&(SessionPurpose::Editor, file)));
}
