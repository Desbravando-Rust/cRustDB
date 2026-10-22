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
}
