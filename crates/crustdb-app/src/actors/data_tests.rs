#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::sync::atomic::Ordering;

use crustdb_core::dialect::BuiltQuery;
use crustdb_core::driver::{Param, SessionPurpose};
use crustdb_core::error::{DriverError, ErrorKind};
use crustdb_testkit::fake_driver::FakeScript;

use super::tests::{
    H, TAB, connect, connects, fetch, harness, next, open_table, page, query, query_for_export,
    rows,
};
use crate::runtime::{Command, Event, RequestId};

const SQL: &str = r#"UPDATE "t" SET "v" = $1 WHERE "id" = $2"#;

fn update(h: &mut H, req: u64) {
    assert!(
        h.actors
            .dispatch(Command::UpdateCell {
                tab: TAB,
                req: RequestId(req),
                update: BuiltQuery {
                    sql: SQL.into(),
                    params: vec![Param(Some("x".into())), Param(Some("1".into()))],
                },
            })
            .is_none()
    );
}

fn editors(h: &H) -> usize {
    connects(h)
        .iter()
        .filter(|(p, _)| *p == SessionPurpose::Editor)
        .count()
}

async fn ready(h: &mut H) {
    connect(h, Some("d")).await;
    open_table(h).await;
    fetch(h, 1, query_for_export());
    page(h).await;
}

#[tokio::test]
async fn update_cell_opens_editor_session_and_emits_updated() {
    // Arrange
    let mut h = harness(FakeScript {
        rows: rows(1),
        ..FakeScript::default()
    });
    ready(&mut h).await;

    // Act & Assert
    assert_eq!(editors(&h), 0, "FetchPage must not open a writable session");
    *h.drv.log.affected.lock().unwrap() = Some(1);
    update(&mut h, 2);
    assert!(matches!(
        next(&h.rx).await,
        Event::CellUpdated {
            req: RequestId(2),
            error: None
        }
    ));
    update(&mut h, 3);
    assert!(matches!(
        next(&h.rx).await,
        Event::CellUpdated {
            req: RequestId(3),
            error: None
        }
    ));
    assert_eq!(editors(&h), 1, "the Editor session is reused");
    assert!(h.drv.log.ops().contains(&format!("start:{SQL}")));
}

#[tokio::test]
async fn update_cell_zero_rows_reports_row_not_found() {
    // Arrange
    let mut h = harness(FakeScript {
        rows: rows(1),
        ..FakeScript::default()
    });
    ready(&mut h).await;
    *h.drv.log.affected.lock().unwrap() = Some(0);

    // Act
    update(&mut h, 2);

    // Assert
    assert!(matches!(
        next(&h.rx).await,
        Event::CellUpdated { req: RequestId(2), error: Some(e) } if e == "row not found"
    ));
}

async fn failed_update(h: &mut H, req: u64) {
    update(h, req);
    assert!(matches!(
        next(&h.rx).await,
        Event::CellUpdated { error: Some(_), .. }
    ));
}

#[tokio::test]
async fn connection_error_drops_writer_and_next_update_reconnects() {
    // Arrange
    let mut h = harness(FakeScript {
        rows: rows(1),
        ..FakeScript::default()
    });
    ready(&mut h).await;
    h.drv.log.drop_next.store(true, Ordering::SeqCst);

    // Act & Assert
    failed_update(&mut h, 2).await;
    update(&mut h, 3);
    assert!(matches!(
        next(&h.rx).await,
        Event::CellUpdated { error: None, .. }
    ));
    assert_eq!(editors(&h), 2);
}

#[tokio::test]
async fn query_error_keeps_writer() {
    // Arrange
    let mut h = harness(FakeScript {
        rows: rows(1),
        ..FakeScript::default()
    });
    ready(&mut h).await;
    let e = DriverError {
        kind: ErrorKind::Query {
            code: None,
            position: None,
        },
        message: "bad value".into(),
    };
    h.drv
        .log
        .fail_sql
        .lock()
        .unwrap()
        .push(("UPDATE".into(), e));

    // Act & Assert
    failed_update(&mut h, 2).await;
    failed_update(&mut h, 3).await;
    assert_eq!(editors(&h), 1);
}

#[tokio::test]
async fn fetch_page_sends_tab_limit_verbatim() {
    // Arrange
    let mut h = harness(FakeScript {
        rows: rows(3),
        ..FakeScript::default()
    });
    connect(&mut h, None).await;
    open_table(&mut h).await;

    // Act
    fetch(&mut h, 1, query(3, vec![]));
    page(&mut h).await;

    // Assert
    let sql = super::editor::tests::starts(&h).pop().unwrap();
    assert!(sql.contains("LIMIT 3 "), "{sql}");
}
