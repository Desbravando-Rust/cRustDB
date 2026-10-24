#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::sync::atomic::Ordering;

use crustdb_core::error::{DriverError, ErrorKind};
use crustdb_core::model::DriverKind;
use crustdb_testkit::fake_driver::FakeDriver;

use super::super::tests::{H, TAB, WS, config, harness, harness_with, next};
use super::tests::{
    cmd, evaluate, execute, notices, open, open_editor, run, script, settle, starts, tx_states,
};
use crate::runtime::{Command, Event, RequestId, ResolvedSecrets, TxOp, TxState};

fn query_err(h: &H, pat: &str) {
    h.drv.log.fail_sql.lock().unwrap().push((
        pat.into(),
        DriverError {
            kind: ErrorKind::Query {
                code: None,
                position: None,
            },
            message: "boom".into(),
        },
    ));
}

#[tokio::test(start_paused = true)]
async fn evaluation_does_not_survive_database_change_or_connection_loss() {
    // Arrange
    let mut h = harness(script(1));
    open(&mut h).await;
    evaluate(&mut h, 1, &["DELETE FROM t"]);
    settle(&h).await;
    cmd(
        &mut h,
        Command::UseDatabase {
            tab: TAB,
            database: "other".into(),
        },
    );

    // Act & Assert
    execute(&mut h, 2, 1);
    assert_eq!(notices(&settle(&h).await).len(), 1);
    assert!(starts(&h).is_empty());
    // Connection loss.
    run(&mut h, 3, &["SELECT 1"]).await;
    evaluate(&mut h, 4, &["SELECT 2"]);
    settle(&h).await;
    h.drv.log.drop_next.store(true, Ordering::SeqCst);
    cmd(
        &mut h,
        Command::Tx {
            tab: TAB,
            op: TxOp::Begin,
        },
    );
    let ev = settle(&h).await;
    assert!(
        notices(&ev)
            .iter()
            .any(|n| n.starts_with("Connection lost"))
    );
    let n = starts(&h).len();
    execute(&mut h, 6, 4);
    assert_eq!(notices(&settle(&h).await).len(), 1);
    assert_eq!(starts(&h).len(), n);
}

#[tokio::test(start_paused = true)]
async fn failed_commit_or_rollback_leaves_idle() {
    // Arrange
    let mut h = harness(script(1));
    open(&mut h).await;
    run(&mut h, 1, &["BEGIN"]).await;
    query_err(&h, "COMMIT");

    // Act & Assert
    let ev = run(&mut h, 2, &["COMMIT"]).await;
    assert_eq!(tx_states(&ev), [TxState::Idle]);
    run(&mut h, 3, &["BEGIN"]).await;
    cmd(
        &mut h,
        Command::Tx {
            tab: TAB,
            op: TxOp::Commit,
        },
    );
    assert_eq!(tx_states(&settle(&h).await), [TxState::Idle]);
}

#[tokio::test(start_paused = true)]
async fn mysql_failed_ddl_in_open_tx_is_implicit_commit() {
    // Arrange
    let mut h = harness_with(FakeDriver::with_kind(script(0), DriverKind::Mysql));
    let mut cfg = config(Some("d"));
    cfg.driver = DriverKind::Mysql;

    // Act & Assert
    h.actors.dispatch(Command::Connect {
        ws: WS,
        config: cfg,
        secrets: ResolvedSecrets::default(),
    });
    assert!(matches!(next(&h.rx).await, Event::Connected { .. }));
    open_editor(&mut h);
    run(&mut h, 1, &["BEGIN"]).await;
    query_err(&h, "CREATE");
    let ev = run(&mut h, 2, &["CREATE TABLE x (a int)"]).await;
    assert_eq!(tx_states(&ev), [TxState::Idle]);
    assert!(notices(&ev).iter().any(|n| n.contains("implicit commit")));
}

#[tokio::test(start_paused = true)]
async fn load_more_error_in_open_pg_tx_aborts_it() {
    // Arrange
    let mut h = harness(script(1500));
    open(&mut h).await;
    run(&mut h, 1, &["BEGIN"]).await;
    run(&mut h, 2, &["SELECT 1"]).await;
    h.drv.log.fail_fetch.store(true, Ordering::SeqCst);

    // Act
    cmd(
        &mut h,
        Command::LoadMore {
            tab: TAB,
            req: RequestId(3),
        },
    );

    // Assert
    assert_eq!(tx_states(&settle(&h).await), [TxState::Aborted]);
}

#[tokio::test(start_paused = true)]
async fn failure_to_open_session_is_not_a_lost_connection() {
    // Arrange
    let mut h = harness(script(1));
    open(&mut h).await;
    h.drv.log.fail_connects.store(1, Ordering::SeqCst);

    // Act & Assert
    let ev = run(&mut h, 1, &["SELECT 1"]).await;
    assert!(notices(&ev).is_empty());
    assert!(tx_states(&ev).is_empty());
    let h0 = ev.iter().find_map(|e| match e {
        Event::ExecError { history, .. } => Some(history.clone()),
        _ => None,
    });
    assert!(h0.unwrap().error.unwrap().starts_with("not sent"));
}

#[tokio::test(start_paused = true)]
async fn mysql_ddl_parse_error_keeps_tx_open() {
    // Arrange
    let mut h = harness_with(FakeDriver::with_kind(script(0), DriverKind::Mysql));
    let mut cfg = config(Some("d"));
    cfg.driver = DriverKind::Mysql;

    // Act & Assert
    h.actors.dispatch(Command::Connect {
        ws: WS,
        config: cfg,
        secrets: ResolvedSecrets::default(),
    });
    assert!(matches!(next(&h.rx).await, Event::Connected { .. }));
    open_editor(&mut h);
    run(&mut h, 1, &["BEGIN"]).await;
    h.drv.log.fail_sql.lock().unwrap().push((
        "intt".into(),
        DriverError {
            kind: ErrorKind::Query {
                code: Some("1064".into()),
                position: None,
            },
            message: "syntax".into(),
        },
    ));
    let ev = run(&mut h, 2, &["CREATE TABLE x (a intt)"]).await;
    assert!(tx_states(&ev).is_empty());
    assert!(!notices(&ev).iter().any(|n| n.contains("implicit commit")));
}
