#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::sync::atomic::Ordering;
use std::time::Duration;

use crustdb_testkit::fake_driver::FakeScript;

use super::tests::{WS, connect, drain, harness, next};
use crate::runtime::{Command, Event, RequestId};

fn load(h: &mut super::tests::H, req: u64, database: &str) {
    h.actors.dispatch(Command::LoadDiagram {
        ws: WS,
        req: RequestId(req),
        database: database.into(),
        schema: Some("public".into()),
    });
}

#[tokio::test]
async fn load_diagram_answers_with_the_schema_tables() {
    let mut h = harness(FakeScript::default());
    connect(&mut h, Some("a")).await;
    load(&mut h, 9, "a");
    assert!(matches!(
        next(&h.rx).await,
        Event::Diagram { req: RequestId(9), tables: Some(t) } if t.is_empty()
    ));
}

#[tokio::test]
async fn load_diagram_failure_answers_none_with_notice() {
    let mut h = harness(FakeScript::default());
    connect(&mut h, Some("a")).await;
    h.drv.log.fail_connects.store(1, Ordering::SeqCst);
    load(&mut h, 10, "b");
    tokio::time::sleep(Duration::from_millis(50)).await;
    let ev = drain(&h.rx);
    assert!(ev.iter().any(|e| matches!(e, Event::Notice { .. })));
    assert!(ev.iter().any(|e| matches!(
        e,
        Event::Diagram {
            req: RequestId(10),
            tables: None
        }
    )));
}
