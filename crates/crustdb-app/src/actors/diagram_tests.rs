#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::sync::atomic::Ordering;

use crustdb_core::model::{ObjectKind, ObjectRef};
use crustdb_testkit::fake_driver::FakeScript;

use super::tests::{WS, col, connect, harness, next};
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
async fn load_diagram_fetches_columns_of_each_table() {
    let mut h = harness(FakeScript {
        columns: vec![col("id")],
        ..Default::default()
    });
    h.drv.log.tables.lock().unwrap().push(ObjectRef {
        database: "a".into(),
        schema: Some("public".into()),
        name: "t".into(),
        kind: ObjectKind::Table,
    });
    connect(&mut h, Some("a")).await;
    load(&mut h, 11, "a");
    let Event::Diagram {
        tables: Some(t), ..
    } = next(&h.rx).await
    else {
        panic!("expected a loaded diagram");
    };
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].object.name, "t");
    assert_eq!(t[0].columns[0].name, "id");
}

#[tokio::test]
async fn load_diagram_failure_answers_none_with_notice() {
    let mut h = harness(FakeScript::default());
    connect(&mut h, Some("a")).await;
    h.drv.log.fail_connects.store(1, Ordering::SeqCst);
    load(&mut h, 10, "b");
    let mut seen = vec![];
    let tables = loop {
        match next(&h.rx).await {
            Event::Diagram { req, tables } => {
                assert_eq!(req, RequestId(10));
                break tables;
            }
            e => seen.push(e),
        }
    };
    assert!(tables.is_none());
    assert!(seen.iter().any(|e| matches!(e, Event::Notice { .. })));
}
