//! Inline grid edits staged per row and saved as one `UPDATE` (ADR 0004).
use crustdb_core::edit::{build_update, is_opaque};
use crustdb_core::model::{Cell, ObjectKind};

use super::TableTab;
use crate::runtime::{Command, RequestId};

const PENDING_NOTICE: &str = "Save or discard pending changes";

impl TableTab {
    /// Opens the inline editor on a text or NULL cell of a table with a primary key, showing the
    /// staged value if there is one.
    pub fn begin_edit(&mut self, row: usize, col: usize) {
        let keyed =
            self.object.kind == ObjectKind::Table && self.columns.iter().any(|c| c.is_primary_key);
        let text = match self.rows.get(row).and_then(|r| r.get(col)) {
            Some(c) if is_opaque(c) => return,
            Some(Cell::Text { value, .. }) => value.to_string(),
            Some(Cell::Null) => String::new(),
            _ => return,
        };
        if !keyed || !self.can_stage(row) {
            return;
        }
        let text = match self.pending_value(row, col) {
            Some(v) => v.unwrap_or_default(),
            None => text,
        };
        self.editing = Some((row, col, text));
    }

    /// Closes the inline editor and stages the typed text.
    pub fn commit_edit(&mut self) {
        if let Some((row, col, text)) = self.editing.take() {
            self.stage(row, col, Some(text));
        }
    }

    pub fn set_null(&mut self, row: usize, col: usize) {
        self.editing = None;
        self.stage(row, col, None);
    }

    /// The staged value of a cell: `Some(None)` is a staged NULL.
    pub fn pending_value(&self, row: usize, col: usize) -> Option<Option<String>> {
        (self.pending_row == Some(row))
            .then(|| self.pending.iter().find(|(c, _)| *c == col))
            .flatten()
            .map(|(_, v)| v.clone())
    }

    /// Sends every staged change of the row as one `UpdateCell`; rejections land in `notice`.
    pub fn save_pending(&mut self, req: RequestId) -> Option<Command> {
        if self.pending.is_empty() || self.edit_req.is_some() {
            return None;
        }
        let cells = self.rows.get(self.pending_row?)?;
        let d = crate::export_fmt::dialect(self.driver);
        match build_update(d, &self.object, &self.columns, cells, &self.pending) {
            Ok(update) => {
                self.edit_req = Some(req);
                Some(Command::UpdateCell {
                    tab: self.id,
                    req,
                    update,
                })
            }
            Err(e) => {
                self.notice = Some(e.to_string());
                None
            }
        }
    }

    pub fn discard_pending(&mut self) {
        self.pending.clear();
        self.pending_row = None;
        if self.notice.as_deref() == Some(PENDING_NOTICE) {
            self.notice = None;
        }
    }

    /// Success drops the staged changes and reloads the page; a failure keeps them for another
    /// Save and only shows `error` (an in-flight load stays pending).
    pub fn cell_updated(&mut self, req: RequestId, error: Option<String>) {
        if self.edit_req != Some(req) {
            return;
        }
        self.edit_req = None;
        match error {
            None => {
                self.discard_pending();
                self.needs_fetch = true;
            }
            Some(e) => self.notice = Some(e),
        }
    }

    /// Paging, sorting, filtering and reloading would drop staged changes: refuse with a notice.
    pub(crate) fn pending_blocks(&mut self) -> bool {
        let blocked = !self.pending.is_empty();
        if blocked {
            self.notice = Some(PENDING_NOTICE.into());
        }
        blocked
    }

    /// One row at a time, and not while a page load or a save is in flight (row indexes
    /// would go stale).
    fn can_stage(&mut self, row: usize) -> bool {
        if self.loading.is_some() || self.edit_req.is_some() {
            return false;
        }
        if self.pending_row.is_some_and(|r| r != row) {
            self.notice = Some(PENDING_NOTICE.into());
            return false;
        }
        true
    }

    /// A value equal to the original is not staged (and un-stages that column); re-editing a
    /// column replaces its staged value. Rejections (no key, binary or truncated cells) land in
    /// `notice` right away instead of on Save.
    fn stage(&mut self, row: usize, col: usize, value: Option<String>) {
        let Some(cells) = self.rows.get(row) else {
            return;
        };
        let unchanged = match (cells.get(col), &value) {
            (Some(Cell::Null), None) => true,
            (Some(Cell::Text { value: old, .. }), Some(new)) => **old == **new,
            _ => false,
        };
        let d = crate::export_fmt::dialect(self.driver);
        // ponytail: builds a throwaway one-column UPDATE just to reuse its validation.
        let check = build_update(d, &self.object, &self.columns, cells, &[(col, None)]);
        if let Err(e) = check {
            self.notice = Some(e.to_string());
            return;
        }
        if !self.can_stage(row) {
            return;
        }
        self.pending.retain(|(c, _)| *c != col);
        if !unchanged {
            self.pending.push((col, value));
        }
        self.pending_row = (!self.pending.is_empty()).then_some(row);
    }
}

#[cfg(test)]
#[path = "table_tab_edit_tests.rs"]
mod tests;
