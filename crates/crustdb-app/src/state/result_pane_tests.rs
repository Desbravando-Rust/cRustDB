#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crustdb_core::model::{Cell, ColumnMeta, Completion, TypeCategory};

use crate::storage::history::HistoryEntry;

pub(crate) fn hist(rows: Option<u64>) -> HistoryEntry {
    HistoryEntry {
        ts: String::new(),
        database: None,
        sql: String::new(),
        duration_ms: 0,
        rows,
        ok: true,
        error: None,
    }
}

pub(crate) fn header() -> ResultHeader {
    ResultHeader {
        columns: vec![ColumnMeta {
            name: "a".into(),
            type_name: "int".into(),
            category: TypeCategory::Integer,
            nullable: None,
            is_primary_key: false,
        }],
    }
}

pub(crate) fn rows(n: usize) -> Vec<Row> {
    (0..n).map(|_| vec![Cell::Null].into()).collect()
}

pub(crate) fn done(req: u64, index: usize, affected: Option<u64>, rows: Option<u64>) -> Event {
    Event::StatementDone {
        req: RequestId(req),
        index,
        completion: Completion {
            affected_rows: affected,
        },
        duration: Duration::from_millis(5),
        history: hist(rows),
    }
}

#[test]
fn grid_shows_last_statement_with_rows() {
    // Arrange
    let mut p = ResultPane::default();
    let r = RequestId(7);
    p.apply(&Event::ResultStarted {
        req: r,
        header: header(),
    });
    p.apply(&Event::Rows {
        req: r,
        rows: rows(3),
        exhausted: true,
    });
    p.apply(&done(7, 0, None, Some(3)));

    // Act & Assert
    // The second statement returns nothing: the grid keeps the first one's rows.
    p.apply(&done(7, 1, Some(2), Some(2)));
    assert_eq!(p.rows.len(), 3);
    assert!(p.header.is_some());
    // A later statement with rows replaces it.
    let mut p = ResultPane::default();
    p.apply(&Event::ResultStarted {
        req: r,
        header: header(),
    });
    p.apply(&Event::Rows {
        req: r,
        rows: rows(3),
        exhausted: true,
    });
    p.apply(&Event::ResultStarted {
        req: r,
        header: header(),
    });
    p.apply(&Event::Rows {
        req: r,
        rows: rows(1),
        exhausted: true,
    });
    assert_eq!(p.rows.len(), 1);
}

fn text(v: &str) -> Cell {
    Cell::Text {
        value: v.into(),
        truncated: false,
    }
}

fn col(name: &str, category: TypeCategory) -> ColumnMeta {
    ColumnMeta {
        name: name.into(),
        type_name: String::new(),
        category,
        nullable: None,
        is_primary_key: false,
    }
}

/// Pane with columns `n` (Integer) and `s` (Text) loaded with `rows`.
fn loaded(rows: Vec<Row>) -> ResultPane {
    loaded_with(TypeCategory::Integer, rows)
}

fn loaded_with(cat: TypeCategory, rows: Vec<Row>) -> ResultPane {
    let mut p = ResultPane::default();
    let r = RequestId(1);
    p.apply(&Event::ResultStarted {
        req: r,
        header: ResultHeader {
            columns: vec![col("n", cat), col("s", TypeCategory::Text)],
        },
    });
    p.apply(&Event::Rows {
        req: r,
        rows,
        exhausted: false,
    });
    p
}

fn row2(n: Cell, s: &str) -> Row {
    vec![n, text(s)].into()
}

#[test]
fn view_lists_all_rows_without_sort_or_filter() {
    // Act
    let p = loaded(vec![row2(text("1"), "a"), row2(text("2"), "b")]);

    // Assert
    assert_eq!(p.view, vec![0, 1]);
}

#[test]
fn numeric_sort_cycles_asc_desc_off() {
    // Arrange
    let mut p = loaded(vec![
        row2(text("10"), "a"),
        row2(text("9"), "b"),
        row2(text("100"), "c"),
    ]);

    // Act & Assert
    p.click_header("n", false);
    assert_eq!(p.view, vec![1, 0, 2]);
    p.click_header("n", false);
    assert_eq!(p.view, vec![2, 0, 1]);
    p.click_header("n", false);
    assert!(p.sorts.is_empty());
    assert_eq!(p.view, vec![0, 1, 2]);
}

#[test]
fn nulls_sort_last_both_ways() {
    // Arrange
    let mut p = loaded(vec![
        row2(text("2"), "a"),
        row2(Cell::Null, "b"),
        row2(text("1"), "c"),
    ]);

    // Act & Assert
    p.click_header("n", false);
    assert_eq!(p.view, vec![2, 0, 1]);
    p.click_header("n", false);
    assert_eq!(p.view, vec![0, 2, 1]);
}

#[test]
fn filter_matches_any_text_cell_case_insensitive() {
    // Arrange
    let mut p = loaded(vec![
        row2(text("1"), "Apple"),
        row2(Cell::Null, "banana"),
        row2(text("31"), "cherry"),
    ]);

    // Act & Assert
    p.filter = "AN".into();
    p.refresh_view();
    assert_eq!(p.view, vec![1]);
    p.filter = "1".into();
    p.refresh_view();
    assert_eq!(p.view, vec![0, 2]);
    p.filter.clear();
    p.refresh_view();
    assert_eq!(p.view, vec![0, 1, 2]);
}

#[test]
fn more_rows_keep_sort_and_filter() {
    // Arrange
    let mut p = loaded(vec![row2(text("5"), "x"), row2(text("3"), "y")]);
    p.click_header("n", false);
    p.filter = "x".into();

    // Act & Assert
    p.refresh_view();
    assert_eq!(p.view, vec![0]);
    p.apply(&Event::Rows {
        req: RequestId(1),
        rows: vec![row2(text("1"), "xx")],
        exhausted: true,
    });
    assert_eq!(p.view, vec![2, 0]);
}

#[test]
fn result_started_resets_sort() {
    // Arrange
    let mut p = loaded(vec![row2(text("2"), "a"), row2(text("1"), "b")]);

    // Act & Assert
    p.click_header("s", true);
    assert_eq!(p.sorts.len(), 1);
    p.apply(&Event::ResultStarted {
        req: RequestId(1),
        header: header(),
    });
    assert!(p.sorts.is_empty());
    assert!(p.view.is_empty());
}

#[test]
fn numeric_column_puts_non_numbers_after_numbers() {
    // Arrange
    let mut p = loaded(vec![
        row2(text("abc"), "a"),
        row2(text("10"), "b"),
        row2(text("9"), "c"),
        row2(text("Z"), "d"),
    ]);

    // Act & Assert
    p.click_header("n", false);
    assert_eq!(p.view, vec![2, 1, 3, 0]);
    p.click_header("n", false);
    assert_eq!(p.view, vec![0, 3, 1, 2]);
}

#[test]
fn short_row_sorts_as_null_last() {
    // Arrange
    let short: Row = vec![text("1")].into();
    let mut p = loaded(vec![short, row2(text("2"), "b"), row2(text("3"), "a")]);

    // Act & Assert
    p.click_header("s", false);
    assert_eq!(p.view, vec![2, 1, 0]);
    p.click_header("s", false);
    assert_eq!(p.view, vec![1, 2, 0]);
}

#[test]
fn integer_sort_is_exact_above_2_pow_53() {
    // Arrange
    let mut p = loaded(vec![
        row2(text("9007199254740993"), "a"),
        row2(text("9007199254740992"), "b"),
    ]);

    // Act
    p.click_header("n", false);

    // Assert
    assert_eq!(p.view, vec![1, 0]);
}

#[test]
fn decimal_sort_is_exact_beyond_f64() {
    // Arrange
    let mut p = loaded_with(
        TypeCategory::Decimal,
        vec![
            row2(text("1.00000000000000001"), "a"),
            row2(text("1.0"), "b"),
        ],
    );

    // Act & Assert
    p.click_header("n", false);
    assert_eq!(p.view, vec![1, 0]);

    let mut p = loaded_with(
        TypeCategory::Decimal,
        vec![
            row2(text("0"), "a"),
            row2(text("1.5"), "b"),
            row2(text("-1.5"), "c"),
            row2(text("1.50"), "d"),
            row2(text("-2"), "e"),
            row2(text("-0"), "f"),
        ],
    );
    p.click_header("n", false);
    assert_eq!(p.view, vec![4, 2, 0, 5, 1, 3]);
}

#[test]
fn untrimmed_non_numbers_keep_total_order() {
    // Arrange
    let mut p = loaded(vec![
        row2(text(""), "a"),
        row2(text(" 7"), "b"),
        row2(text("8"), "c"),
    ]);

    // Act
    p.click_header("n", false);

    // Assert
    assert_eq!(p.view, vec![2, 0, 1]);
}

#[test]
fn decimal_accepts_leading_dot() {
    // Arrange
    let mut p = loaded_with(
        TypeCategory::Decimal,
        vec![
            row2(text("0.5"), "a"),
            row2(text(".5"), "b"),
            row2(text(".25"), "c"),
        ],
    );

    // Act
    p.click_header("n", false);

    // Assert
    assert_eq!(p.view, vec![2, 0, 1]);
}

#[test]
fn exec_error_switches_to_messages() {
    // Arrange
    let mut p = ResultPane::default();

    // Act
    p.apply(&Event::ExecError {
        req: RequestId(1),
        index: 0,
        error: crustdb_core::error::DriverError {
            kind: crustdb_core::error::ErrorKind::Protocol,
            message: "boom".into(),
        },
        history: hist(None),
    });

    // Assert
    assert_eq!(p.tab, ResultTab::Messages);
}

#[test]
fn statement_done_without_result_switches_to_messages() {
    // Arrange
    let mut p = ResultPane::default();

    // Act
    p.apply(&done(1, 0, Some(0), None));

    // Assert
    assert_eq!(p.tab, ResultTab::Messages);
}

#[test]
fn result_started_keeps_result_tab() {
    // Arrange
    let mut p = ResultPane {
        tab: ResultTab::Messages,
        ..Default::default()
    };
    let r = RequestId(1);

    // Act
    p.apply(&Event::ResultStarted {
        req: r,
        header: header(),
    });
    p.apply(&Event::Rows {
        req: r,
        rows: rows(1),
        exhausted: true,
    });
    p.apply(&done(1, 0, None, Some(1)));

    // Assert
    assert_eq!(p.tab, ResultTab::Result);
}
