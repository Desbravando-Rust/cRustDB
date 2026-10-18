#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

fn tab(text: &str) -> QueryTab {
    let mut t = QueryTab::new(TabId(1), DriverKind::Postgres, None);
    t.text = text.into();
    t
}

#[test]
fn run_current_uses_statement_under_cursor() {
    let t = tab("select 1; select 2;");
    assert_eq!(t.targets(12, None, false), vec!["select 2"]);
    assert_eq!(t.targets(3, Some(3..3), false), vec!["select 1"]);
}

#[test]
fn run_with_selection_runs_statements_inside_selection() {
    let t = tab("select 1; select 2; select 3;");
    assert_eq!(
        t.targets(0, Some(10..29), false),
        vec!["select 2", "select 3"]
    );
}

#[test]
fn partial_selection_runs_only_selected_text() {
    let t = tab("delete from a;\nselect 2");
    // Selection starts at `a` and crosses the `;`.
    assert_eq!(t.targets(0, Some(12..23), false), vec!["a", "select 2"]);
}

#[test]
fn run_all_returns_every_statement() {
    let t = tab("select 1; select 2; select 3");
    assert_eq!(t.targets(0, None, true).len(), 3);
}

#[test]
fn char_to_byte_offset_roundtrip() {
    let s = "ação🦀x";
    assert_eq!(char_to_byte(s, 0), 0);
    assert_eq!(char_to_byte(s, 2), 3);
    assert_eq!(char_to_byte(s, 4), 6);
    assert_eq!(char_to_byte(s, 5), 10);
    assert_eq!(&s[char_to_byte(s, 4)..], "🦀x");
    assert_eq!(char_to_byte(s, 99), s.len());
}

#[test]
fn toggle_comment_adds_and_removes_dash_dash() {
    let t = "select 1\n  select 2\nselect 3";
    let on = toggle_comment(t, 0..2);
    assert_eq!(on, "-- select 1\n--   select 2\nselect 3");
    assert_eq!(toggle_comment(&on, 0..2), t);
}

#[test]
fn error_position_maps_to_line() {
    let s = "ação;\nselect x";
    // 'x' is the 14th char (1-based) on line 1.
    assert_eq!(error_line(s, 14), 1);
    assert_eq!(error_line(s, 1), 0);
    let mut t = tab(s);
    t.set_error(14);
    assert_eq!(t.error_marker, Some((1, 15..16)));
}

use crate::state::result_pane::tests::{done, header, hist, rows};
use crustdb_core::error::DriverError;

fn started(t: &mut QueryTab, req: u64) {
    t.starts = vec![0, 0];
    t.begin_run(RequestId(req));
}

#[test]
fn messages_list_every_statement_with_affected_rows() {
    let mut t = tab("");
    started(&mut t, 7);
    t.apply(&done(7, 0, Some(4), Some(4)));
    t.apply(&Event::ExecError {
        req: RequestId(7),
        index: 1,
        error: DriverError {
            kind: ErrorKind::Query {
                code: None,
                position: Some(3),
            },
            message: "boom".into(),
        },
        history: hist(None),
    });
    let m = &t.pane.messages;
    assert_eq!(m.len(), 2);
    assert!(m[0].ok && m[0].text.contains("4 rows affected"));
    assert!(!m[1].ok && m[1].text.contains("boom") && m[1].index == 1);
    assert_eq!(t.running, None);
    assert!(t.error_marker.is_some());
}

#[test]
fn running_spans_the_batch_and_stop_clears_it() {
    let mut t = tab("");
    started(&mut t, 7);
    assert_eq!(t.running, Some(RequestId(7)));
    t.apply(&done(7, 0, None, Some(1)));
    assert_eq!(t.running, Some(RequestId(7)));
    t.apply(&done(7, 1, None, Some(1)));
    assert_eq!(t.running, None);
    started(&mut t, 8);
    assert!(matches!(t.stop(), Some(Command::CancelTab { .. })));
    assert_eq!(t.running, None);
    assert!(t.stop().is_none());
    started(&mut t, 9);
    t.apply(&Event::Notice {
        tab: t.id,
        text: "implicit commit: x".into(),
    });
    assert!(t.running.is_some());
    t.apply(&Event::Notice {
        tab: t.id,
        text: "Connection lost — open transaction was rolled back by the server".into(),
    });
    assert_eq!(t.running, None);
}

#[test]
fn load_more_hidden_when_exhausted() {
    let mut t = tab("");
    started(&mut t, 7);
    let r = RequestId(7);
    t.apply(&Event::ResultStarted {
        req: r,
        header: header(),
    });
    t.apply(&Event::Rows {
        req: r,
        rows: rows(2),
        exhausted: false,
    });
    t.apply(&done(7, 1, None, Some(2)));
    assert!(t.pane.load_more_visible());
    assert!(matches!(t.load_more(), Some(Command::LoadMore { .. })));
    t.apply(&Event::Rows {
        req: r,
        rows: rows(1),
        exhausted: true,
    });
    assert!(!t.pane.load_more_visible());
    assert!(t.load_more().is_none());
    assert_eq!(t.pane.rows.len(), 3);
}

#[test]
fn error_position_is_offset_by_statement_start() {
    let mut t = tab("select 1;\nselect x");
    t.starts = vec![0, 10];
    t.begin_run(RequestId(7));
    t.apply(&Event::ExecError {
        req: RequestId(7),
        index: 1,
        error: DriverError {
            kind: ErrorKind::Query {
                code: None,
                position: Some(8),
            },
            message: "boom".into(),
        },
        history: hist(None),
    });
    assert_eq!(t.error_marker.map(|m| m.0), Some(1));
}

#[test]
fn stale_evaluation_notice_clears_running() {
    let mut t = tab("");
    started(&mut t, 7);
    t.apply(&Event::Notice {
        tab: t.id,
        text: STALE_EVALUATION.into(),
    });
    assert_eq!(t.running, None);
}

#[test]
fn load_more_and_export_are_busy_stoppable_and_report() {
    let mut t = tab("");
    started(&mut t, 7);
    let r = RequestId(7);
    t.apply(&Event::ResultStarted {
        req: r,
        header: header(),
    });
    t.apply(&Event::Rows {
        req: r,
        rows: rows(1),
        exhausted: false,
    });
    t.apply(&done(7, 1, None, Some(1)));
    assert!(t.load_more().is_some() && t.running.is_some() && !t.can_run());
    assert!(matches!(t.stop(), Some(Command::CancelTab { .. })));
    t.export_path = "/tmp/x.csv".into();
    assert!(matches!(t.export(), Some(Command::ExportResult { .. })));
    assert!(t.running.is_some() && t.export().is_none());
    t.apply(&Event::ExportDone {
        req: r,
        result: Err("disk full".into()),
    });
    assert_eq!(t.running, None);
    let m = t.pane.messages.last().unwrap();
    assert!(!m.ok && m.text.contains("disk full"));
    assert!(t.export().is_some());
    t.apply(&Event::ExportDone {
        req: r,
        result: Ok(3),
    });
    assert!(t.pane.messages.last().unwrap().text.contains("3 rows"));
}

#[test]
fn run_disabled_while_running_stop_enabled() {
    let mut t = tab("");
    assert!(t.can_run() && !t.can_stop());
    started(&mut t, 7);
    assert!(!t.can_run() && t.can_stop());
}

#[test]
fn db_selector_disabled_with_open_tx_on_pg() {
    let mut t = tab("");
    assert!(t.can_switch_db());
    t.tx = TxState::Open;
    assert!(!t.can_switch_db());
    assert!(t.use_database("x".into()).is_none());
    t.driver = DriverKind::Mysql;
    assert!(t.can_switch_db());
    assert!(matches!(
        t.use_database("x".into()),
        Some(Command::UseDatabase { .. })
    ));
    assert_eq!(t.database.as_deref(), Some("x"));
}

#[test]
fn tx_badge_state_follows_event() {
    let mut t = tab("");
    t.apply(&Event::TxChanged {
        tab: t.id,
        state: TxState::Aborted,
    });
    assert_eq!(t.tx, TxState::Aborted);
    t.apply(&Event::TxChanged {
        tab: t.id,
        state: TxState::Idle,
    });
    assert_eq!(t.tx, TxState::Idle);
}

#[test]
fn highlight_disabled_above_1mib() {
    assert!(highlight_enabled("select 1"));
    assert!(!highlight_enabled(&"a".repeat(HIGHLIGHT_MAX_BYTES + 1)));
}

#[test]
fn current_statement_range_for_background() {
    let t = tab("select 1;\n  select 2;");
    assert_eq!(t.current_range(15), Some(12..20));
}
