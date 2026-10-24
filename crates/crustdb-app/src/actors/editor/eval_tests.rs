#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::sync::atomic::Ordering;

use crustdb_core::safety::{FlagReason, Verdict};
use crustdb_testkit::fake_driver::FakeScript;

use super::super::tests::{H, harness};
use super::tests::{evaluate, open, ops, script, settle};
use crate::runtime::Event;

async fn evaluated(h: &mut H, sql: &[&str]) -> crustdb_core::safety::Evaluation {
    evaluate(h, 1, sql);
    settle(h)
        .await
        .into_iter()
        .find_map(|e| match e {
            Event::Evaluated { evaluation, .. } => Some(evaluation),
            _ => None,
        })
        .unwrap()
}

#[tokio::test(start_paused = true)]
async fn evaluate_uses_explain_and_applies_threshold() {
    // Arrange
    let mut h = harness(FakeScript {
        estimate: Some(5000),
        ..script(1)
    });
    open(&mut h).await;

    // Act & Assert
    let e = evaluated(&mut h, &["DELETE FROM t WHERE a = 1"]).await;
    assert_eq!(e.verdict, Verdict::Password);
    assert_eq!(e.flags[0].reason, FlagReason::OverThreshold(5000));
    h.drv.log.fail_explain.store(true, Ordering::SeqCst);
    let e = evaluated(&mut h, &["DELETE FROM t WHERE a = 1"]).await;
    assert_eq!(e.flags[0].reason, FlagReason::UnknownEstimate);
}

#[tokio::test(start_paused = true)]
async fn evaluate_skips_explain_for_values_copy_and_reads() {
    // Arrange
    let mut h = harness(script(1));
    open(&mut h).await;

    // Act & Assert
    let e = evaluated(&mut h, &["INSERT INTO t VALUES (1)", "SELECT 1"]).await;
    assert_eq!(e.verdict, Verdict::Run);
    let e = evaluated(&mut h, &["COPY t FROM '/x'"]).await;
    assert_eq!(e.verdict, Verdict::Password);
    assert!(!ops(&h).iter().any(|x| x.starts_with("explain:")));
}

#[tokio::test(start_paused = true)]
async fn evaluate_unknown_estimate_for_statement_depending_on_earlier_ddl() {
    // Arrange
    let mut h = harness(FakeScript {
        estimate: Some(1),
        ..script(1)
    });
    open(&mut h).await;

    // Act
    let e = evaluated(
        &mut h,
        &["CREATE TABLE x (a int)", "DELETE FROM x WHERE a = 1"],
    )
    .await;

    // Assert
    assert_eq!(e.flags.len(), 1);
    assert_eq!(e.flags[0].index, 1);
    assert_eq!(e.flags[0].reason, FlagReason::UnknownEstimate);
    assert!(!ops(&h).iter().any(|x| x.starts_with("explain:")));
}

#[tokio::test(start_paused = true)]
async fn evaluate_drop_table_estimates_single_table_only() {
    // Arrange
    let mut h = harness(FakeScript {
        estimate: Some(10),
        ..script(1)
    });
    open(&mut h).await;

    // Act & Assert
    let e = evaluated(&mut h, &["DROP TABLE public.a"]).await;
    assert_eq!(e.verdict, Verdict::Confirm);
    assert!(ops(&h).contains(&"estimate:a".to_string()));
    let e = evaluated(&mut h, &["DROP TABLE public.a, public.b"]).await;
    assert_eq!(e.verdict, Verdict::Password);
    assert!(!ops(&h).contains(&"estimate:b".to_string()));
}

async fn assert_no_estimate(sql: &str) {
    let mut h = harness(FakeScript {
        estimate: Some(10),
        ..script(1)
    });
    open(&mut h).await;
    let e = evaluated(&mut h, &[sql]).await;
    assert_eq!(e.verdict, Verdict::Password, "{sql}");
    assert!(!ops(&h).iter().any(|x| x.starts_with("estimate:")), "{sql}");
}

#[tokio::test(start_paused = true)]
async fn cascade_and_unqualified_pg_tables_have_unknown_estimate() {
    // Act & Assert
    assert_no_estimate("TRUNCATE public.a CASCADE").await;
    assert_no_estimate("drop table public.a cascade").await;
    assert_no_estimate("DROP TABLE a").await; // search_path decides on PG
}

#[tokio::test(start_paused = true)]
async fn session_change_or_earlier_dml_invalidates_later_estimates() {
    // Arrange
    let mut h = harness(FakeScript {
        estimate: Some(1),
        ..script(1)
    });
    open(&mut h).await;

    // Act & Assert
    let e = evaluated(
        &mut h,
        &["SET search_path = prod", "DELETE FROM t WHERE a = 1"],
    )
    .await;
    assert_eq!(e.flags[0].reason, FlagReason::UnknownEstimate);
    let e = evaluated(
        &mut h,
        &[
            "DELETE FROM t WHERE a = 1",
            "DELETE FROM t WHERE a = 2",
            "DELETE FROM u WHERE a = 2",
        ],
    )
    .await;
    let unknown: Vec<_> = e
        .flags
        .iter()
        .filter(|f| f.reason == FlagReason::UnknownEstimate)
        .map(|f| f.index)
        .collect();
    assert_eq!(unknown, [1]);
}

#[tokio::test(start_paused = true)]
async fn failed_explain_in_open_pg_tx_aborts_it() {
    // Arrange
    let mut h = harness(script(1));
    open(&mut h).await;
    super::tests::run(&mut h, 9, &["BEGIN"]).await;
    h.drv.log.fail_explain.store(true, Ordering::SeqCst);

    // Act
    evaluate(&mut h, 1, &["DELETE FROM t WHERE a = 1"]);

    // Assert
    let ev = settle(&h).await;
    assert!(ev.iter().any(|e| matches!(
        e,
        Event::TxChanged {
            state: crate::runtime::TxState::Aborted,
            ..
        }
    )));
    assert!(ev.iter().any(|e| matches!(
        e,
        Event::Notice { text, .. } if text.contains("failed EXPLAIN")
    )));
}

#[tokio::test(start_paused = true)]
async fn table_names_match_case_insensitively_and_unknown_invalidates() {
    // Arrange
    let mut h = harness(FakeScript {
        estimate: Some(1),
        ..script(1)
    });
    open(&mut h).await;

    // Act & Assert
    let e = evaluated(
        &mut h,
        &["DELETE FROM T WHERE a = 1", "DELETE FROM t WHERE a = 2"],
    )
    .await;
    assert_eq!(e.flags.len(), 1);
    assert_eq!(
        (e.flags[0].index, e.flags[0].reason),
        (1, FlagReason::UnknownEstimate)
    );
    let e = evaluated(
        &mut h,
        &[
            "SELECT set_config('search_path', 'prod', false)",
            "DELETE FROM t WHERE a = 1",
        ],
    )
    .await;
    assert!(
        e.flags
            .iter()
            .any(|f| f.index == 1 && f.reason == FlagReason::UnknownEstimate)
    );
}
