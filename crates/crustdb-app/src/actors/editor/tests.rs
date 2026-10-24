#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::sync::atomic::Ordering;
use std::time::Duration;

use crustdb_core::error::{DriverError, ErrorKind};
use crustdb_core::model::DriverKind;
use crustdb_testkit::fake_driver::{FakeDriver, FakeScript};

use super::super::tests::{
    H, TAB, WS, col, config, connect, drain, harness, harness_with, next, rows,
};
use crate::runtime::{Command, Event, RequestId, ResolvedSecrets, TxOp, TxState};

pub(in crate::actors) fn script(n: usize) -> FakeScript {
    FakeScript {
        columns: vec![col("id")],
        rows: rows(n),
        ..FakeScript::default()
    }
}

pub(in crate::actors) async fn open(h: &mut H) {
    connect(h, Some("d")).await;
    open_editor(h);
}

pub(in crate::actors) fn open_editor(h: &mut H) {
    assert!(
        h.actors
            .dispatch(Command::OpenEditor {
                ws: WS,
                tab: TAB,
                database: None,
            })
            .is_none()
    );
}

pub(in crate::actors) async fn settle(h: &H) -> Vec<Event> {
    tokio::time::sleep(Duration::from_millis(50)).await;
    drain(&h.rx)
}

pub(in crate::actors) fn evaluate(h: &mut H, req: u64, sql: &[&str]) {
    assert!(
        h.actors
            .dispatch(Command::Evaluate {
                tab: TAB,
                req: RequestId(req),
                statements: sql.iter().map(|s| s.to_string()).collect(),
            })
            .is_none()
    );
}

pub(in crate::actors) fn execute(h: &mut H, req: u64, evaluation: u64) {
    assert!(
        h.actors
            .dispatch(Command::Execute {
                tab: TAB,
                req: RequestId(req),
                evaluation: RequestId(evaluation),
            })
            .is_none()
    );
}

/// Evaluate + Execute with the same id.
pub(in crate::actors) async fn run(h: &mut H, req: u64, sql: &[&str]) -> Vec<Event> {
    evaluate(h, req, sql);
    execute(h, req + 1000, req);
    settle(h).await
}

pub(in crate::actors) fn cmd(h: &mut H, c: Command) {
    assert!(h.actors.dispatch(c).is_none());
}

pub(in crate::actors) fn ops(h: &H) -> Vec<String> {
    h.drv.log.ops()
}

pub(in crate::actors) fn starts(h: &H) -> Vec<String> {
    ops(h)
        .iter()
        .filter_map(|o| o.strip_prefix("start:").map(String::from))
        .collect()
}

pub(in crate::actors) fn notices(ev: &[Event]) -> Vec<&str> {
    ev.iter()
        .filter_map(|e| match e {
            Event::Notice { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

pub(in crate::actors) fn tx_states(ev: &[Event]) -> Vec<TxState> {
    ev.iter()
        .filter_map(|e| match e {
            Event::TxChanged { state, .. } => Some(*state),
            _ => None,
        })
        .collect()
}

fn row_counts(ev: &[Event]) -> Vec<(usize, bool)> {
    ev.iter()
        .filter_map(|e| match e {
            Event::Rows {
                rows, exhausted, ..
            } => Some((rows.len(), *exhausted)),
            _ => None,
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn evaluate_finishes_paused_cursor_before_explain() {
    // Arrange
    let mut h = harness(script(1500));
    open(&mut h).await;
    run(&mut h, 1, &["SELECT 1"]).await;

    // Act
    evaluate(&mut h, 2, &["DELETE FROM t WHERE id = 1"]);
    let ev = settle(&h).await;

    // Assert
    assert!(matches!(ev.last(), Some(Event::Evaluated { .. })));
    let o = ops(&h);
    let fin = o.iter().rposition(|x| x == "finish").unwrap();
    let exp = o.iter().position(|x| x.starts_with("explain:")).unwrap();
    assert!(fin < exp, "{o:?}");
}

#[tokio::test(start_paused = true)]
async fn evaluate_of_plain_select_keeps_paused_cursor() {
    // Arrange
    let mut h = harness(script(1500));
    open(&mut h).await;
    run(&mut h, 1, &["SELECT 1"]).await;

    // Act
    evaluate(&mut h, 2, &["SELECT 2"]);
    settle(&h).await;

    // Assert
    assert!(!ops(&h).iter().any(|x| x == "finish"));
}

#[tokio::test(start_paused = true)]
async fn execute_rejects_unknown_evaluation_id() {
    // Arrange
    let mut h = harness(script(1));
    open(&mut h).await;

    // Act & Assert
    execute(&mut h, 9, 5);
    let ev = settle(&h).await;
    assert_eq!(notices(&ev).len(), 1);
    assert!(starts(&h).is_empty());
    // An evaluation is single-use.
    evaluate(&mut h, 5, &["SELECT 1"]);
    execute(&mut h, 6, 5);
    settle(&h).await;
    execute(&mut h, 7, 5);
    let ev = settle(&h).await;
    assert_eq!(notices(&ev).len(), 1);
    assert_eq!(starts(&h), ["SELECT 1"]);
}

#[tokio::test(start_paused = true)]
async fn execute_runs_only_the_evaluated_statements() {
    // Arrange
    let mut h = harness(script(1));
    open(&mut h).await;

    // Act
    evaluate(&mut h, 1, &["SELECT 1"]);
    evaluate(&mut h, 2, &["SELECT 2"]);
    execute(&mut h, 3, 1); // superseded by the newer Evaluate
    execute(&mut h, 4, 2);
    settle(&h).await;

    // Assert
    assert_eq!(starts(&h), ["SELECT 2"]);
}

#[tokio::test(start_paused = true)]
async fn confirm_flow_cancel_executes_nothing() {
    // Arrange
    let mut h = harness(script(1));
    open(&mut h).await;

    // Act
    evaluate(&mut h, 1, &["DELETE FROM t"]);
    let ev = settle(&h).await;

    // Assert
    assert!(matches!(
        ev.last(),
        Some(Event::Evaluated {
            req: RequestId(1),
            ..
        })
    ));
    assert!(starts(&h).is_empty());
}

#[tokio::test(start_paused = true)]
async fn run_all_stops_at_first_error() {
    // Arrange
    let mut h = harness(script(1));
    h.drv.log.fail_sql.lock().unwrap().push((
        "bad".into(),
        DriverError {
            kind: ErrorKind::Query {
                code: None,
                position: None,
            },
            message: "boom".into(),
        },
    ));
    open(&mut h).await;

    // Act
    let ev = run(&mut h, 1, &["SELECT 1", "SELECT bad", "SELECT 3"]).await;

    // Assert
    assert_eq!(starts(&h).len(), 2);
    let err = ev.iter().find_map(|e| match e {
        Event::ExecError { index, history, .. } => Some((*index, history.ok)),
        _ => None,
    });
    assert_eq!(err, Some((1, false)));
}

#[tokio::test(start_paused = true)]
async fn only_last_statement_keeps_cursor() {
    // Arrange
    let mut h = harness(script(1500));
    open(&mut h).await;

    // Act
    let ev = run(&mut h, 1, &["SELECT a", "SELECT b"]).await;

    // Assert
    // Statement a: read to the limit, then closed; statement b: left paused.
    let o = ops(&h);
    assert_eq!(o.iter().filter(|x| *x == "finish").count(), 1, "{o:?}");
    assert_ne!(o.last().unwrap(), "finish");
    let r = row_counts(&ev);
    assert_eq!(r.iter().map(|x| x.0).sum::<usize>(), 2000);
    // a is closed (exhausted), b stays paused.
    assert!(r[1].1 && !r[3].1);
}

#[tokio::test(start_paused = true)]
async fn load_more_fetches_editor_row_limit() {
    // Arrange
    let mut h = harness(script(2500));
    open(&mut h).await;

    // Act & Assert
    let ev = run(&mut h, 1, &["SELECT 1"]).await;
    assert_eq!(row_counts(&ev), [(500, false), (500, false)]);
    cmd(
        &mut h,
        Command::LoadMore {
            tab: TAB,
            req: RequestId(2),
        },
    );
    let ev = settle(&h).await;
    assert_eq!(row_counts(&ev), [(500, false), (500, false)]);
    cmd(
        &mut h,
        Command::LoadMore {
            tab: TAB,
            req: RequestId(3),
        },
    );
    let ev = settle(&h).await;
    assert_eq!(row_counts(&ev), [(500, true)]);
    assert_eq!(ops(&h).last().unwrap(), "finish");
    // Nothing paused any more.
    cmd(
        &mut h,
        Command::LoadMore {
            tab: TAB,
            req: RequestId(4),
        },
    );
    assert_eq!(notices(&settle(&h).await).len(), 1);
}

#[tokio::test(start_paused = true)]
async fn idle_paused_cursor_auto_closed() {
    // Arrange
    let mut h = harness(script(1500));
    open(&mut h).await;
    run(&mut h, 1, &["SELECT 1"]).await;

    // Act & Assert
    tokio::time::sleep(Duration::from_secs(299)).await;
    assert!(notices(&drain(&h.rx)).is_empty()); // pings are not notices
    assert!(!ops(&h).iter().any(|x| x == "finish"));
    tokio::time::sleep(Duration::from_secs(2)).await;
    let ev = drain(&h.rx);
    assert_eq!(notices(&ev).len(), 1);
    assert_eq!(ops(&h).last().unwrap(), "finish");
}

#[tokio::test(start_paused = true)]
async fn tx_state_follows_buttons_and_statements() {
    // Arrange
    let mut h = harness(script(1));
    open(&mut h).await;
    let tx = |h: &mut H, op| cmd(h, Command::Tx { tab: TAB, op });

    // Act & Assert
    tx(&mut h, TxOp::Begin);
    assert_eq!(tx_states(&settle(&h).await), [TxState::Open]);
    h.drv.log.fail_sql.lock().unwrap().push((
        "bad".into(),
        DriverError {
            kind: ErrorKind::Query {
                code: None,
                position: None,
            },
            message: "boom".into(),
        },
    ));
    let ev = run(&mut h, 1, &["SELECT bad"]).await;
    assert_eq!(tx_states(&ev), [TxState::Aborted]);
    let ev = run(&mut h, 2, &["ROLLBACK"]).await;
    assert_eq!(tx_states(&ev), [TxState::Idle]);
    let ev = run(&mut h, 3, &["BEGIN"]).await;
    assert_eq!(tx_states(&ev), [TxState::Open]);
    tx(&mut h, TxOp::Commit);
    assert_eq!(tx_states(&settle(&h).await), [TxState::Idle]);
    tx(&mut h, TxOp::Savepoint);
    assert_eq!(notices(&settle(&h).await).len(), 1);
}

#[tokio::test(start_paused = true)]
async fn mysql_ddl_in_open_tx_is_implicit_commit() {
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

    // Act & Assert
    let ev = run(&mut h, 2, &["CREATE TABLE x (a int)"]).await;
    assert_eq!(tx_states(&ev), [TxState::Idle]);
    assert!(notices(&ev).iter().any(|n| n.contains("implicit commit")));
    // MySQL errors do not abort the transaction.
    run(&mut h, 3, &["BEGIN"]).await;
    h.drv.log.fail_sql.lock().unwrap().push((
        "bad".into(),
        DriverError {
            kind: ErrorKind::Query {
                code: None,
                position: None,
            },
            message: "boom".into(),
        },
    ));
    let ev = run(&mut h, 4, &["SELECT bad"]).await;
    assert!(tx_states(&ev).is_empty());
}

#[tokio::test(start_paused = true)]
async fn use_database_blocked_with_open_tx_on_pg() {
    // Arrange
    let mut h = harness(script(1));
    open(&mut h).await;
    run(&mut h, 1, &["BEGIN"]).await;
    let before = h.drv.log.connects.lock().unwrap().len();

    // Act & Assert
    cmd(
        &mut h,
        Command::UseDatabase {
            tab: TAB,
            database: "other".into(),
        },
    );
    assert_eq!(notices(&settle(&h).await).len(), 1);
    assert_eq!(h.drv.log.connects.lock().unwrap().len(), before);
    run(&mut h, 2, &["ROLLBACK"]).await;
    cmd(
        &mut h,
        Command::UseDatabase {
            tab: TAB,
            database: "other".into(),
        },
    );
    settle(&h).await;
    run(&mut h, 3, &["SELECT 1"]).await;
    let c = h.drv.log.connects.lock().unwrap();
    assert_eq!(c.last().unwrap().1.as_deref(), Some("other"));
}

#[tokio::test(start_paused = true)]
async fn close_tab_with_open_tx_requires_resolution() {
    // Arrange
    let mut h = harness(script(1));
    open(&mut h).await;
    run(&mut h, 1, &["BEGIN"]).await;

    // Act & Assert
    cmd(
        &mut h,
        Command::CloseTab {
            tab: TAB,
            resolve: None,
        },
    );
    assert_eq!(notices(&settle(&h).await).len(), 1);
    cmd(
        &mut h,
        Command::CloseTab {
            tab: TAB,
            resolve: Some(TxOp::Rollback),
        },
    );
    settle(&h).await;
    assert_eq!(starts(&h).last().unwrap(), "ROLLBACK");
    // The tab is gone.
    assert!(
        h.actors
            .dispatch(Command::Tx {
                tab: TAB,
                op: TxOp::Begin
            })
            .is_some()
    );
}

#[tokio::test(start_paused = true)]
async fn close_idle_tab_just_closes() {
    // Arrange
    let mut h = harness(script(1));
    open(&mut h).await;
    run(&mut h, 1, &["SELECT 1"]).await;

    // Act
    cmd(
        &mut h,
        Command::CloseTab {
            tab: TAB,
            resolve: None,
        },
    );
    settle(&h).await;

    // Assert
    assert!(
        h.actors
            .dispatch(Command::CloseTab {
                tab: TAB,
                resolve: None
            })
            .is_some()
    );
}

#[tokio::test(start_paused = true)]
async fn history_entry_emitted_per_statement() {
    // Arrange
    let mut h = harness(script(3));
    open(&mut h).await;

    // Act
    let ev = run(&mut h, 1, &["SELECT 1", "SELECT 2"]).await;

    // Assert
    let hist: Vec<_> = ev
        .iter()
        .filter_map(|e| match e {
            Event::StatementDone { index, history, .. } => Some((*index, history.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(hist.len(), 2);
    assert_eq!(hist[1].0, 1);
    assert_eq!(hist[1].1.sql, "SELECT 2");
    assert_eq!(hist[1].1.database.as_deref(), Some("d"));
    assert!(hist[1].1.ok);
    assert_eq!(hist[1].1.rows, Some(3));
}

#[tokio::test(start_paused = true)]
async fn editor_connection_lost_reports_and_reconnects_on_next_run() {
    // Arrange
    let mut h = harness(script(1));
    open(&mut h).await;
    run(&mut h, 1, &["BEGIN"]).await;
    h.drv.log.drop_next.store(true, Ordering::SeqCst);

    // Act & Assert
    let ev = run(&mut h, 2, &["SELECT 1"]).await;
    assert!(
        notices(&ev).contains(&"Connection lost — open transaction was rolled back by the server")
    );
    assert_eq!(tx_states(&ev), [TxState::Idle]);
    let n = h.drv.log.connects.lock().unwrap().len();
    let ev = run(&mut h, 3, &["SELECT 1"]).await;
    assert!(ev.iter().any(|e| matches!(e, Event::StatementDone { .. })));
    assert_eq!(h.drv.log.connects.lock().unwrap().len(), n + 1);
    assert_eq!(
        h.drv.log.connects.lock().unwrap().last().unwrap().0,
        crustdb_core::driver::SessionPurpose::Editor
    );
}

#[tokio::test(start_paused = true)]
async fn cancel_tab_cancels_running_statement() {
    // Arrange
    let mut h = harness(FakeScript {
        delay: Duration::from_secs(60),
        ..script(1)
    });
    open(&mut h).await;

    // Act
    evaluate(&mut h, 1, &["SELECT 1"]);
    execute(&mut h, 2, 1);
    tokio::time::sleep(Duration::from_secs(1)).await;
    cmd(&mut h, Command::CancelTab { tab: TAB });
    let ev = settle(&h).await;

    // Assert
    assert!(ev.iter().any(|e| matches!(
        e,
        Event::ExecError { error, .. } if error.kind == ErrorKind::Cancelled
    )));
}
