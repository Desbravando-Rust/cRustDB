//! Result pane of a query tab: last grid, Messages lines and tab selection.
use std::cmp::Ordering;
use std::time::Duration;

use crustdb_core::driver::ResultHeader;
use crustdb_core::model::{Cell, Row, TypeCategory};
use crustdb_core::query::Sort;

use super::table_tab::toggle_sort;
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
    /// Index into `rows`, not into `view`.
    pub selected: Option<usize>,
    /// Other end of a Shift+click range; `selected` is the cursor.
    pub select_anchor: Option<usize>,
    /// Client-side sort of the loaded rows.
    pub sorts: Vec<Sort>,
    pub filter: String,
    /// Indexes into `rows` that pass `filter`, in `sorts` order.
    pub view: Vec<usize>,
}

impl ResultPane {
    pub fn apply(&mut self, e: &Event) {
        match e {
            Event::ResultStarted { header, .. } => {
                self.header = Some(header.clone());
                self.tab = ResultTab::Result;
                self.rows.clear();
                self.exhausted = false;
                self.selected = None;
                self.select_anchor = None;
                self.sorts.clear();
                self.view.clear();
            }
            Event::Rows {
                rows, exhausted, ..
            } => {
                self.rows.extend(rows.iter().cloned());
                self.exhausted = *exhausted;
                self.refresh_view();
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
                // No grid to show (DDL, USE, INSERT): surface the message.
                if self.header.is_none() {
                    self.tab = ResultTab::Messages;
                }
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
        // Errors must never stay hidden behind the grid.
        if !ok {
            self.tab = ResultTab::Messages;
        }
    }

    pub fn click_header(&mut self, column: &str, shift: bool) {
        toggle_sort(&mut self.sorts, column, shift);
        self.refresh_view();
    }

    /// Rebuilds `view` from `rows`, `filter` and `sorts`.
    pub fn refresh_view(&mut self) {
        // ponytail: lowercases every cell per keystroke, O(rows*cols); cache lowercase per row if it lags
        let needle = self.filter.to_lowercase();
        let rows = &self.rows;
        self.view = (0..rows.len())
            .filter(|&i| {
                needle.is_empty()
                    || rows[i].iter().any(|c| match c {
                        Cell::Text { value, .. } => value.to_lowercase().contains(&needle),
                        _ => false,
                    })
            })
            .collect();
        let Some(h) = &self.header else { return };
        let keys: Vec<(usize, TypeCategory, bool)> = self
            .sorts
            .iter()
            .filter_map(|s| {
                let c = h.columns.iter().position(|c| c.name == s.column)?;
                Some((c, h.columns[c].category, s.descending))
            })
            .collect();
        if keys.is_empty() {
            return;
        }
        self.view.sort_by(|&a, &b| {
            keys.iter()
                .map(|&(c, cat, desc)| cmp_cells(rows[a].get(c), rows[b].get(c), cat, desc))
                .find(|o| o.is_ne())
                .unwrap_or(Ordering::Equal)
        });
    }

    pub fn load_more_visible(&self) -> bool {
        self.header.is_some() && !self.exhausted
    }
}

/// Missing cell (row shorter than the header) sorts as null.
static MISSING: Cell = Cell::Null;

// ponytail: Infinity/NaN/exponent forms and numbers > 309 digits fall back to f64 and may not be
// transitive against the exact path; add a full decimal parser if that ever shows up.
/// Splits `[+-]?(\d+(\.\d*)?|\.\d+)` into (negative, int digits, fraction digits), normalized; zero is positive.
fn split_decimal(s: &str) -> Option<(bool, &str, &str)> {
    let (neg, s) = match s.as_bytes().first()? {
        b'-' => (true, &s[1..]),
        b'+' => (false, &s[1..]),
        _ => (false, s),
    };
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    if (int.is_empty() && frac.is_empty())
        || !int.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let (int, frac) = (int.trim_start_matches('0'), frac.trim_end_matches('0'));
    Some((neg && !(int.is_empty() && frac.is_empty()), int, frac))
}

/// Exact decimal comparison without f64; `None` if either side is not a plain decimal literal.
fn cmp_exact(x: &str, y: &str) -> Option<Ordering> {
    let ((nx, ix, fx), (ny, iy, fy)) = (split_decimal(x)?, split_decimal(y)?);
    if nx != ny {
        return Some(if nx {
            Ordering::Less
        } else {
            Ordering::Greater
        });
    }
    let mag = ix
        .len()
        .cmp(&iy.len())
        .then_with(|| ix.cmp(iy))
        .then_with(|| fx.cmp(fy));
    Some(if nx { mag.reverse() } else { mag })
}

/// Nulls last in both directions; numbers by value (Integer/Decimal exactly, Float via f64)
/// before non-numeric text, the rest as text/bytes.
fn cmp_cells(a: Option<&Cell>, b: Option<&Cell>, cat: TypeCategory, desc: bool) -> Ordering {
    let o = match (a.unwrap_or(&MISSING), b.unwrap_or(&MISSING)) {
        (Cell::Null, Cell::Null) => return Ordering::Equal,
        (Cell::Null, _) => return Ordering::Greater,
        (_, Cell::Null) => return Ordering::Less,
        (Cell::Text { value: x, .. }, Cell::Text { value: y, .. }) => {
            let numeric = matches!(
                cat,
                TypeCategory::Integer | TypeCategory::Decimal | TypeCategory::Float
            );
            let exact = matches!(cat, TypeCategory::Integer | TypeCategory::Decimal)
                .then(|| cmp_exact(x, y))
                .flatten();
            if let Some(o) = exact {
                o
            } else if numeric {
                match (x.parse::<f64>(), y.parse::<f64>()) {
                    (Ok(x), Ok(y)) => x.total_cmp(&y),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => x.cmp(y),
                }
            } else {
                x.cmp(y)
            }
        }
        (Cell::Bytes { value: x, .. }, Cell::Bytes { value: y, .. }) => x.cmp(y),
        (Cell::Text { .. }, Cell::Bytes { .. }) => Ordering::Less,
        (Cell::Bytes { .. }, Cell::Text { .. }) => Ordering::Greater,
    };
    if desc { o.reverse() } else { o }
}

#[cfg(test)]
#[path = "result_pane_tests.rs"]
pub(crate) mod tests;
