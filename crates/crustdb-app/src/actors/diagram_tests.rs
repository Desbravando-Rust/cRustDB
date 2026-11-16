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
    // Arrange
    let mut h = harness(FakeScript::default());
    connect(&mut h, Some("a")).await;

    // Act
    load(&mut h, 9, "a");

    // Assert
    assert!(matches!(
        next(&h.rx).await,
        Event::Diagram { req: RequestId(9), tables: Ok(t) } if t.is_empty()
    ));
}

#[tokio::test]
async fn load_diagram_fetches_columns_of_each_table() {
    // Arrange
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

    // Act
    load(&mut h, 11, "a");

    // Assert
    let Event::Diagram { tables: Ok(t), .. } = next(&h.rx).await else {
        panic!("expected a loaded diagram");
    };
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].object.name, "t");
    assert_eq!(t[0].columns[0].name, "id");
}

#[tokio::test]
async fn load_diagram_failure_answers_with_the_error() {
    // Arrange
    let mut h = harness(FakeScript::default());
    connect(&mut h, Some("a")).await;
    h.drv.log.fail_connects.store(1, Ordering::SeqCst);
    load(&mut h, 10, "b");

    // Act
    let tables = loop {
        if let Event::Diagram { req, tables } = next(&h.rx).await {
            assert_eq!(req, RequestId(10));
            break tables;
        }
    };

    // Assert
    assert!(tables.is_err_and(|e| !e.is_empty()));
}
