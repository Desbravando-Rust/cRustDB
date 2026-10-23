#![allow(clippy::unwrap_used, clippy::expect_used)]
use crustdb_core::dialect::BuiltQuery;
use crustdb_core::driver::{Param, SessionPurpose};
use crustdb_testkit::fake_driver::FakeScript;

use super::tests::{
    H, TAB, connect, connects, fetch, harness, next, open_table, page, query_for_export, rows,
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
    let mut h = harness(FakeScript {
        rows: rows(1),
        ..FakeScript::default()
    });
    ready(&mut h).await;
    assert_eq!(editors(&h), 0, "FetchPage must not open a writable session");
    *h.drv.log.affected.lock().unwrap() = Some(1);
    update(&mut h, 2);
    assert!(matches!(
        next(&h.rx).await,
        Event::CellUpdated { req: RequestId(2) }
    ));
    update(&mut h, 3);
    assert!(matches!(
        next(&h.rx).await,
        Event::CellUpdated { req: RequestId(3) }
    ));
    assert_eq!(editors(&h), 1, "the Editor session is reused");
    assert!(h.drv.log.ops().contains(&format!("start:{SQL}")));
}

#[tokio::test]
async fn update_cell_zero_rows_emits_notice() {
    let mut h = harness(FakeScript {
        rows: rows(1),
        ..FakeScript::default()
    });
    ready(&mut h).await;
    *h.drv.log.affected.lock().unwrap() = Some(0);
    update(&mut h, 2);
    assert!(matches!(
        next(&h.rx).await,
        Event::Notice { text, .. } if text == "row not found"
    ));
}
