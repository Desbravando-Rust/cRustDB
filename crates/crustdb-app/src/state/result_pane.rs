//! Result pane of a query tab: last grid, Messages lines and tab selection.
use std::time::Duration;

use crustdb_core::driver::ResultHeader;
use crustdb_core::model::Row;

use crate::runtime::{Event, RequestId};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ResultTab {
    #[default]
    Result,
    Messages,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageLine {
    pub index: usize,
    pub ok: bool,
    pub text: String,
    pub duration: Duration,
}

/// Output of the last run: grid of the last statement with rows (D19) plus one line per statement.
#[derive(Debug, Default)]
pub struct ResultPane {
    pub header: Option<ResultHeader>,
    pub rows: Vec<Row>,
    pub exhausted: bool,
    pub messages: Vec<MessageLine>,
    pub tab: ResultTab,
    /// Execute request whose events this pane shows.
    pub req: Option<RequestId>,
    pub selected: Option<usize>,
}

impl ResultPane {
    pub fn apply(&mut self, e: &Event) {
        match e {
            Event::ResultStarted { header, .. } => {
                self.header = Some(header.clone());
                self.rows.clear();
                self.exhausted = false;
                self.selected = None;
            }
            Event::Rows {
                rows, exhausted, ..
            } => {
                self.rows.extend(rows.iter().cloned());
                self.exhausted = *exhausted;
            }
            Event::StatementDone {
                index,
                completion,
                duration,
                history,
                ..
            } => {
                let text = match (completion.affected_rows, history.rows) {
                    (Some(n), _) => format!("{n} rows affected"),
                    (None, Some(n)) => format!("{n} rows returned"),
                    (None, None) => "OK".into(),
                };
                self.push(*index, true, text, *duration);
            }
            Event::ExecError { index, error, .. } => {
                self.push(*index, false, error.message.clone(), Duration::ZERO);
            }
            _ => {}
        }
    }

    pub(crate) fn push(&mut self, index: usize, ok: bool, text: String, duration: Duration) {
        self.messages.push(MessageLine {
            index,
            ok,
            text,
            duration,
        });
    }

    pub fn load_more_visible(&self) -> bool {
        self.header.is_some() && !self.exhausted
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub(crate) mod tests {
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
        let mut p = ResultPane::default();
        let r = RequestId(1);
        p.apply(&Event::ResultStarted {
            req: r,
            header: ResultHeader {
                columns: vec![
                    col("n", TypeCategory::Integer),
                    col("s", TypeCategory::Text),
                ],
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
        let p = loaded(vec![row2(text("1"), "a"), row2(text("2"), "b")]);
        assert_eq!(p.view, vec![0, 1]);
    }

    #[test]
    fn numeric_sort_cycles_asc_desc_off() {
        let mut p = loaded(vec![
            row2(text("10"), "a"),
            row2(text("9"), "b"),
            row2(text("100"), "c"),
        ]);
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
        let mut p = loaded(vec![
            row2(text("2"), "a"),
            row2(Cell::Null, "b"),
            row2(text("1"), "c"),
        ]);
        p.click_header("n", false);
        assert_eq!(p.view, vec![2, 0, 1]);
        p.click_header("n", false);
        assert_eq!(p.view, vec![0, 2, 1]);
    }

    #[test]
    fn filter_matches_any_text_cell_case_insensitive() {
        let mut p = loaded(vec![
            row2(text("1"), "Apple"),
            row2(Cell::Null, "banana"),
            row2(text("31"), "cherry"),
        ]);
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
        let mut p = loaded(vec![row2(text("5"), "x"), row2(text("3"), "y")]);
        p.click_header("n", false);
        p.filter = "x".into();
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
        let mut p = loaded(vec![row2(text("2"), "a"), row2(text("1"), "b")]);
        p.click_header("s", true);
        assert_eq!(p.sorts.len(), 1);
        p.apply(&Event::ResultStarted {
            req: RequestId(1),
            header: header(),
        });
        assert!(p.sorts.is_empty());
        assert!(p.view.is_empty());
    }
}
