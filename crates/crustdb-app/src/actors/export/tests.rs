#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Duration;

use crustdb_core::driver::SessionPurpose;
use crustdb_core::model::Cell;
use crustdb_core::query::{Filter, Operator, Sort, TableQuery};
use crustdb_testkit::fake_driver::FakeScript;

use super::super::editor::tests::{
    cmd, notices, open, open_editor, run, script, settle, tx_states,
};
use super::super::tests::{H, TAB, WS, col, connect, harness, query_for_export, rows};
use crate::runtime::{Command, Event, RequestId};

fn export(h: &mut H, req: u64, q: TableQuery, path: &Path) {
    cmd(
        h,
        Command::ExportTable {
            tab: TAB,
            req: RequestId(req),
            query: q,
            path: path.to_path_buf(),
        },
    );
}

async fn open_table_tab(h: &mut H) {
    connect(h, None).await;
    cmd(
        h,
        Command::OpenTable {
            ws: WS,
            tab: TAB,
            object: query_for_export().object,
        },
    );
}

/// Waits for `ExportDone`; returns it with every event seen before.
async fn done(h: &H) -> (Result<u64, String>, Vec<Event>) {
    let mut seen = Vec::new();
    for _ in 0..30_000 {
        while let Ok(e) = h.rx.try_recv() {
            if let Event::ExportDone { result, .. } = e {
                return (result, seen);
            }
            seen.push(e);
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("no ExportDone");
}

fn big(n: usize) -> FakeScript {
    FakeScript {
        columns: vec![col("id")],
        rows: rows(n),
        ..FakeScript::default()
    }
}

fn part(path: &Path) -> std::path::PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(".part");
    p.into()
}

#[tokio::test]
async fn export_full_table_streams_csv() {
    let mut h = harness(big(250_000));
    open_table_tab(&mut h).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.csv");
    export(&mut h, 7, query_for_export(), &path);
    let (res, seen) = done(&h).await;
    assert_eq!(res, Ok(250_000));
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().count(), 250_001);
    assert!(text.starts_with("id\n0\n1\n"));
    let progress: Vec<u64> = seen
        .iter()
        .filter_map(|e| match e {
            Event::ExportProgress { req, rows } if *req == RequestId(7) => Some(*rows),
            _ => None,
        })
        .collect();
    assert_eq!(progress.len(), 50);
    assert_eq!(progress[0], 5_000);
    assert!(!part(&path).exists());
}

#[tokio::test]
async fn export_respects_filters_and_sorts() {
    let mut h = harness(big(3));
    open_table_tab(&mut h).await;
    let dir = tempfile::tempdir().unwrap();
    let mut q = query_for_export();
    q.limit = Some(2);
    q.offset = 4;
    q.filters = vec![Filter {
        column: "id".into(),
        op: Operator::Eq,
        values: vec!["1".into()],
    }];
    q.sorts = vec![Sort {
        column: "id".into(),
        descending: true,
    }];
    export(&mut h, 1, q, &dir.path().join("t.csv"));
    assert_eq!(done(&h).await.0, Ok(3));
    let sql = super::super::editor::tests::starts(&h).remove(0);
    assert!(sql.contains("WHERE") && sql.contains("ORDER BY"), "{sql}");
    assert!(!sql.contains("LIMIT") && !sql.contains("OFFSET"), "{sql}");
}

#[tokio::test]
async fn export_uses_export_purpose_session() {
    let mut h = harness(big(1));
    open_table_tab(&mut h).await;
    let dir = tempfile::tempdir().unwrap();
    export(&mut h, 1, query_for_export(), &dir.path().join("t.csv"));
    assert_eq!(done(&h).await.0, Ok(1));
    let c = h.drv.log.connects.lock().unwrap();
    assert!(c.iter().any(|(p, _, _)| *p == SessionPurpose::Export));
}

#[tokio::test]
async fn cancel_export_removes_partial_file() {
    let mut h = harness(big(20_000));
    h.drv.log.fetch_delay_ms.store(2_000, Ordering::SeqCst);
    open_table_tab(&mut h).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.csv");
    export(&mut h, 3, query_for_export(), &path);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(part(&path).exists());
    cmd(&mut h, Command::CancelExport { req: RequestId(3) });
    let (res, _) = done(&h).await;
    assert!(res.unwrap_err().contains("cancel"));
    assert!(!part(&path).exists() && !path.exists());
}

#[tokio::test]
async fn export_result_drains_without_reexecuting() {
    let mut h = harness(script(2_500));
    open(&mut h).await;
    run(&mut h, 1, &["SELECT 1"]).await; // 1000 loaded, cursor paused
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("r.csv");
    cmd(
        &mut h,
        Command::ExportResult {
            tab: TAB,
            req: RequestId(9),
            path: path.clone(),
        },
    );
    assert_eq!(done(&h).await.0, Ok(2_500));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap().lines().count(),
        2_501
    );
    let starts = super::super::editor::tests::starts(&h);
    assert_eq!(starts, ["SELECT 1"]);
}

#[tokio::test]
async fn export_result_warns_on_truncated_cells() {
    let mut s = script(1);
    s.rows = vec![
        vec![Cell::Text {
            value: "x".into(),
            truncated: true,
        }]
        .into(),
    ];
    let mut h = harness(s);
    open(&mut h).await;
    run(&mut h, 1, &["SELECT 1"]).await;
    let dir = tempfile::tempdir().unwrap();
    cmd(
        &mut h,
        Command::ExportResult {
            tab: TAB,
            req: RequestId(2),
            path: dir.path().join("r.csv"),
        },
    );
    let (res, seen) = done(&h).await;
    assert_eq!(res, Ok(1));
    assert!(
        notices(&seen).iter().any(|n| n.contains("truncated")),
        "{seen:?}"
    );
}

#[tokio::test]
async fn export_result_without_result_fails() {
    let mut h = harness(script(1));
    connect(&mut h, Some("d")).await;
    open_editor(&mut h);
    let dir = tempfile::tempdir().unwrap();
    cmd(
        &mut h,
        Command::ExportResult {
            tab: TAB,
            req: RequestId(2),
            path: dir.path().join("r.csv"),
        },
    );
    assert!(done(&h).await.0.is_err());
}

fn export_result(h: &mut H, req: u64, path: &Path) {
    cmd(
        h,
        Command::ExportResult {
            tab: TAB,
            req: RequestId(req),
            path: path.to_path_buf(),
        },
    );
}

#[tokio::test(start_paused = true)]
async fn export_of_cut_result_warns_it_is_partial() {
    let mut h = harness(script(1_500));
    open(&mut h).await;
    run(&mut h, 1, &["INSERT INTO t VALUES (1) RETURNING id"]).await;
    let dir = tempfile::tempdir().unwrap();
    export_result(&mut h, 2, &dir.path().join("r.csv"));
    let (res, seen) = done(&h).await;
    assert_eq!(res, Ok(1_000));
    assert!(
        notices(&seen).iter().any(|n| n.contains("Partial")),
        "{seen:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn export_without_result_keeps_pg_transaction_open() {
    let mut h = harness(FakeScript::default());
    open(&mut h).await;
    cmd(
        &mut h,
        Command::Tx {
            tab: TAB,
            op: crate::runtime::TxOp::Begin,
        },
    );
    run(&mut h, 1, &["UPDATE t SET a = 1 WHERE id = 1"]).await;
    let dir = tempfile::tempdir().unwrap();
    export_result(&mut h, 2, &dir.path().join("r.csv"));
    let (res, seen) = done(&h).await;
    assert!(res.is_err());
    assert!(!tx_states(&seen).contains(&crate::runtime::TxState::Aborted));
}

#[tokio::test(start_paused = true)]
async fn export_to_bad_path_leaves_cursor_usable() {
    let mut h = harness(script(2_500));
    open(&mut h).await;
    run(&mut h, 1, &["SELECT 1"]).await;
    let dir = tempfile::tempdir().unwrap();
    export_result(&mut h, 2, &dir.path().join("missing/r.csv"));
    assert!(done(&h).await.0.is_err());
    cmd(
        &mut h,
        Command::LoadMore {
            tab: TAB,
            req: RequestId(1),
        },
    );
    let ev = settle(&h).await;
    let loaded: usize = ev
        .iter()
        .map(|e| match e {
            Event::Rows { rows, .. } => rows.len(),
            _ => 0,
        })
        .sum();
    assert_eq!(loaded, 1_000);
}

#[tokio::test(start_paused = true)]
async fn export_after_cursor_expiry_warns_it_is_partial() {
    let mut h = harness(script(2_500));
    open(&mut h).await;
    run(&mut h, 1, &["SELECT 1"]).await;
    tokio::time::sleep(Duration::from_secs(301)).await;
    let dir = tempfile::tempdir().unwrap();
    export_result(&mut h, 2, &dir.path().join("r.csv"));
    let (res, seen) = done(&h).await;
    assert_eq!(res, Ok(1_000));
    assert!(
        notices(&seen).iter().any(|n| n.contains("Partial")),
        "{seen:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn cancel_export_waiting_in_backlog_is_honoured() {
    let mut s = script(10);
    s.delay = Duration::from_secs(5);
    let mut h = harness(s);
    open(&mut h).await;
    run(&mut h, 1, &["SELECT 1"]).await;
    super::super::editor::tests::evaluate(&mut h, 2, &["SELECT 2"]);
    tokio::time::sleep(Duration::from_secs(6)).await;
    super::super::editor::tests::execute(&mut h, 3, 2);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("r.csv");
    export_result(&mut h, 4, &path);
    cmd(&mut h, Command::CancelExport { req: RequestId(4) });
    let (res, _) = done(&h).await;
    assert_eq!(res, Err("cancelled".into()));
    assert!(!path.exists() && !part(&path).exists());
}

#[test]
fn failed_finish_removes_partial_file() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("out.csv");
    std::fs::create_dir(&target).unwrap(); // the rename onto a directory fails
    let mut sink = super::Sink::create(&target).unwrap();
    sink.write(&rows(2)).unwrap();
    assert!(sink.finish().is_err());
    assert!(!part(&target).exists());
}

#[test]
fn dropped_sink_removes_partial_file() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("out.csv");
    let sink = super::Sink::create(&target).unwrap();
    assert!(part(&target).exists());
    drop(sink);
    assert!(!part(&target).exists());
}
