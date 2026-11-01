#![allow(clippy::unwrap_used, clippy::expect_used)]
use crustdb_core::model::{DriverKind, ObjectKind};
use crustdb_testkit::fake_driver::{FakeDriver, FakeScript};

use super::tests::{H, WS, config, harness, harness_with, next};
use crate::runtime::{Command, Event, RequestId, ResolvedSecrets};

async fn list(mut h: H, driver: DriverKind, database: Option<&str>) -> Event {
    let mut config = config(database);
    config.driver = driver;
    h.actors.dispatch(Command::Connect {
        ws: WS,
        config,
        secrets: ResolvedSecrets::default(),
    });
    assert!(matches!(next(&h.rx).await, Event::Connected { .. }));
    h.actors.dispatch(Command::LoadChildren {
        ws: WS,
        req: RequestId(1),
        database: String::new(),
        schema: None,
        kind: Some(ObjectKind::Database),
    });
    next(&h.rx).await
}

async fn names(h: H, driver: DriverKind, database: Option<&str>) -> Vec<String> {
    match list(h, driver, database).await {
        Event::Children { items, .. } => items.into_iter().map(|o| o.name).collect(),
        other => panic!("unexpected event: {other:?}"),
    }
}

#[tokio::test]
async fn configured_database_lists_only_that_database() {
    // Act
    let n = names(
        harness(FakeScript::default()),
        DriverKind::Postgres,
        Some("fake"),
    )
    .await;

    // Assert
    assert_eq!(n, ["fake"]);
}

#[tokio::test]
async fn no_configured_database_lists_all() {
    // Act
    let n = names(harness(FakeScript::default()), DriverKind::Postgres, None).await;

    // Assert
    assert_eq!(n, ["fake", "other"]);
}

#[tokio::test]
async fn configured_database_missing_reports_error() {
    // Act
    let ev = list(
        harness(FakeScript::default()),
        DriverKind::Postgres,
        Some("nope"),
    )
    .await;

    // Assert
    let Event::ChildrenFailed { error, .. } = ev else {
        panic!("expected ChildrenFailed, got {ev:?}");
    };
    assert!(error.contains("\"nope\""), "{error}");
}

#[tokio::test]
async fn sqlite_file_path_is_not_checked_against_databases() {
    // Arrange
    let h = harness_with(FakeDriver::with_kind(
        FakeScript::default(),
        DriverKind::Sqlite,
    ));

    // Act
    let n = names(h, DriverKind::Sqlite, Some("/tmp/a.db")).await;

    // Assert
    assert_eq!(n, ["fake", "other"]);
}

#[tokio::test]
async fn mysql_database_match_ignores_case() {
    // Arrange
    let h = harness_with(FakeDriver::with_kind(
        FakeScript::default(),
        DriverKind::Mysql,
    ));

    // Act
    let n = names(h, DriverKind::Mysql, Some("FAKE")).await;

    // Assert
    assert_eq!(n, ["fake"]);
}
