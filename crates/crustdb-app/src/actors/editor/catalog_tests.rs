#![allow(clippy::unwrap_used, clippy::expect_used)]
use crustdb_core::model::DriverKind;
use crustdb_testkit::fake_driver::FakeDriver;

use super::super::tests::{H, WS, config, harness, harness_with, next};
use super::tests::{open, open_editor, run, script};
use crate::runtime::{Command, Event, ResolvedSecrets};

fn changed(ev: &[Event]) -> usize {
    ev.iter()
        .filter(|e| matches!(e, Event::CatalogChanged { .. }))
        .count()
}

#[tokio::test(start_paused = true)]
async fn ddl_in_autocommit_emits_catalog_changed() {
    // Arrange
    let mut h = harness(script(1));
    open(&mut h).await;

    // Act
    let ev = run(
        &mut h,
        1,
        &["CREATE TABLE x (a int)", "ALTER TABLE x ADD b int"],
    )
    .await;

    // Assert
    assert_eq!(changed(&ev), 1);
}

#[tokio::test(start_paused = true)]
async fn ddl_in_open_tx_emits_catalog_changed_only_after_commit() {
    // Arrange
    let mut h = harness(script(1));
    open(&mut h).await;
    run(&mut h, 1, &["BEGIN"]).await;

    // Act
    let inside = run(&mut h, 2, &["CREATE TABLE x (a int)"]).await;
    let after = run(&mut h, 3, &["COMMIT"]).await;

    // Assert
    assert_eq!(changed(&inside), 0);
    assert_eq!(changed(&after), 1);
}

#[tokio::test(start_paused = true)]
async fn mysql_ddl_implicit_commit_emits_catalog_changed() {
    // Arrange
    let mut h = harness_with(FakeDriver::with_kind(script(0), DriverKind::Mysql));
    let mut cfg = config(Some("d"));
    cfg.driver = DriverKind::Mysql;
    h.actors.dispatch(Command::Connect {
        ws: WS,
        config: cfg,
        secrets: ResolvedSecrets::default(),
    });
    assert!(matches!(next(&h.rx).await, Event::Connected { .. }));
    open_editor(&mut h);
    run(&mut h, 1, &["BEGIN"]).await;

    // Act
    let ev = run(&mut h, 2, &["CREATE TABLE x (a int)"]).await;

    // Assert
    assert_eq!(changed(&ev), 1);
}

#[tokio::test(start_paused = true)]
async fn select_batch_does_not_emit_catalog_changed() {
    // Arrange
    let mut h: H = harness(script(1));
    open(&mut h).await;

    // Act
    let ev = run(&mut h, 1, &["SELECT 1"]).await;

    // Assert
    assert_eq!(changed(&ev), 0);
}
