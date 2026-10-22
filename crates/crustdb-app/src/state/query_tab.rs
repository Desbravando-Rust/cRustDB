//! Query editor tab: text, run targets and error position mapping (no GUI).
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crustdb_core::error::ErrorKind;
use crustdb_core::model::DriverKind;
use crustdb_core::sql::{split, statement_at};

use super::autocomplete::Autocomplete;
pub use super::result_pane::{MessageLine, ResultPane, ResultTab};
use crate::actors::{LOST, STALE_EVALUATION};
use crate::runtime::{Command, Event, RequestId, TabId, TxOp, TxState};

/// Above this the editor skips highlighting (D40).
pub const HIGHLIGHT_MAX_BYTES: usize = 1 << 20;

#[derive(Debug)]
pub struct QueryTab {
    pub id: TabId,
    pub driver: DriverKind,
    pub text: String,
    pub file: Option<PathBuf>,
    pub dirty: bool,
    pub database: Option<String>,
    pub tx: TxState,
    pub running: Option<RequestId>,
    /// (line index, byte range of the offending character).
    pub error_marker: Option<(usize, Range<usize>)>,
    pub pane: ResultPane,
    /// Char offset in `text` of each statement of the batch being run; the last `StatementDone` ends it.
    pub starts: Vec<usize>,
    /// `running` is a Load more or an export, not a statement batch.
    pub bg: bool,
    pub ac: Autocomplete,
    /// SQL (from the history panel) the editor inserts at its cursor on the next frame.
    pub insert: Option<String>,
    pub db_draft: String,
}

impl QueryTab {
    pub fn new(id: TabId, driver: DriverKind, database: Option<String>) -> Self {
        Self {
            id,
            driver,
            text: String::new(),
            file: None,
            dirty: false,
            database,
            tx: TxState::Idle,
            running: None,
            error_marker: None,
            pane: ResultPane::default(),
            starts: vec![],
            bg: false,
            ac: Autocomplete::default(),
            insert: None,
            db_draft: String::new(),
        }
    }

    /// The Execute for this tab left: a fresh run replaces the previous result.
    pub fn begin_run(&mut self, req: RequestId) {
        self.running = Some(req);
        self.bg = false;
        self.error_marker = None;
        self.pane = ResultPane {
            req: Some(req),
            ..Default::default()
        };
    }

    pub fn can_run(&self) -> bool {
        self.running.is_none()
    }

    pub fn can_stop(&self) -> bool {
        self.running.is_some()
    }

    /// PG switches database by reconnecting, which an open transaction forbids.
    pub fn can_switch_db(&self) -> bool {
        self.running.is_none() && (self.driver == DriverKind::Mysql || self.tx == TxState::Idle)
    }

    fn idle(&mut self) {
        self.running = None;
        self.bg = false;
    }

    /// Clears `running` right away; late events still reach the pane through `pane.req`.
    pub fn stop(&mut self) -> Option<Command> {
        self.running?;
        self.idle();
        Some(Command::CancelTab { tab: self.id })
    }

    pub fn tx(&self, op: TxOp) -> Command {
        Command::Tx { tab: self.id, op }
    }

    pub fn use_database(&mut self, database: String) -> Option<Command> {
        if !self.can_switch_db() || database.is_empty() {
            return None;
        }
        self.database = Some(database.clone());
        Some(Command::UseDatabase {
            tab: self.id,
            database,
        })
    }

    /// Load more and export keep the tab busy (so Stop cancels them) until their result event.
    fn busy(&mut self, req: RequestId) {
        self.running = Some(req);
        self.bg = true;
    }

    pub fn load_more(&mut self) -> Option<Command> {
        let req = self.pane.req.filter(|_| self.can_run())?;
        if !self.pane.load_more_visible() {
            return None;
        }
        self.busy(req);
        Some(Command::LoadMore { tab: self.id, req })
    }

    pub fn export(&mut self, path: PathBuf) -> Option<Command> {
        let req = self.pane.req.filter(|_| self.can_run())?;
        self.pane.header.as_ref()?;
        self.busy(req);
        Some(Command::ExportResult {
            tab: self.id,
            req,
            path,
        })
    }

    /// Tab label: the file name (`*` while dirty) or `Query`.
    pub fn title(&self) -> String {
        let name = self
            .file
            .as_deref()
            .and_then(Path::file_name)
            .map_or_else(|| "Query".to_owned(), |n| n.to_string_lossy().into_owned());
        if self.dirty {
            format!("{name}•")
        } else {
            name
        }
    }

    pub fn save_to(&mut self, path: PathBuf) -> io::Result<()> {
        std::fs::write(&path, &self.text)?;
        self.file = Some(path);
        self.dirty = false;
        Ok(())
    }

    pub fn load_from(&mut self, path: PathBuf) -> io::Result<()> {
        self.text = std::fs::read_to_string(&path)?;
        self.file = Some(path);
        self.dirty = false;
        Ok(())
    }

    /// Inserts `sql` at the char offset `cursor`; returns the cursor after it.
    pub fn insert_at(&mut self, cursor: usize, sql: &str) -> usize {
        let at = char_to_byte(&self.text, cursor);
        self.text.insert_str(at, sql);
        self.dirty = true;
        cursor + sql.chars().count()
    }

    /// Execution-side events for this tab (routing by request/tab is the caller's).
    pub fn apply(&mut self, e: &Event) {
        match e {
            Event::TxChanged { state, .. } => self.tx = *state,
            Event::Notice { text, .. } => {
                let n = self.pane.messages.len();
                self.pane.push(n, true, text.clone(), Duration::ZERO);
                // The actor refused an Execute (stale evaluation) or lost the session.
                if text.starts_with(LOST) || text == STALE_EVALUATION || self.bg {
                    self.idle();
                }
            }
            Event::ActorCrashed { .. } => {
                let n = self.pane.messages.len();
                self.pane
                    .push(n, false, "connection actor crashed".into(), Duration::ZERO);
                self.idle();
            }
            Event::StatementDone { index, .. } => {
                self.pane.apply(e);
                if index + 1 >= self.starts.len() {
                    self.idle();
                }
            }
            Event::Rows { .. } => {
                self.pane.apply(e);
                // ponytail: ends Load more at its first batch; later batches still reach the pane.
                if self.bg {
                    self.idle();
                }
            }
            Event::ExportDone { result, .. } => {
                let (ok, text) = match result {
                    Ok(n) => (true, format!("exported {n} rows")),
                    Err(e) => (false, format!("export failed: {e}")),
                };
                let n = self.pane.messages.len();
                self.pane.push(n, ok, text, Duration::ZERO);
                self.idle();
            }
            Event::ExecError { index, error, .. } => {
                self.pane.apply(e);
                self.idle();
                if let ErrorKind::Query {
                    position: Some(p), ..
                } = &error.kind
                {
                    // `position` counts from the start of the statement that was sent.
                    let start = self.starts.get(*index).copied().unwrap_or(0);
                    self.set_error(start + p);
                }
            }
            _ => self.pane.apply(e),
        }
    }

    /// Statements to evaluate: all, the selection's, or the one under the cursor.
    pub fn targets(
        &self,
        cursor_byte: usize,
        selection: Option<Range<usize>>,
        all: bool,
    ) -> Vec<String> {
        self.target_spans(cursor_byte, selection, all)
            .into_iter()
            .map(|(_, t)| t)
            .collect()
    }

    /// Like `targets`, with each statement's start as a char offset in `text`.
    pub fn target_spans(
        &self,
        cursor_byte: usize,
        selection: Option<Range<usize>>,
        all: bool,
    ) -> Vec<(usize, String)> {
        // A selection runs exactly the selected text, never statements that merely touch it.
        let selection = selection.filter(|r| !r.is_empty());
        let base = selection.as_ref().map_or(0, |r| r.start);
        let spans = match (&selection, all) {
            (Some(r), false) => split(&self.text[r.clone()], self.driver),
            _ => split(&self.text, self.driver),
        };
        let base = if matches!((&selection, all), (Some(_), false)) {
            base
        } else {
            0
        };
        let at = |s: &crustdb_core::sql::StatementSpan| {
            (
                self.text[..base + s.range.start].chars().count(),
                s.text.clone(),
            )
        };
        if all || selection.is_some() {
            return spans.iter().map(at).collect();
        }
        statement_at(&spans, cursor_byte)
            .map(at)
            .into_iter()
            .collect()
    }

    /// Byte range of the statement under the cursor, for the background tint.
    pub fn current_range(&self, cursor_byte: usize) -> Option<Range<usize>> {
        let spans = split(&self.text, self.driver);
        statement_at(&spans, cursor_byte).map(|s| s.range.clone())
    }

    /// Marks a server error position (1-based chars) and returns nothing if out of range.
    pub fn set_error(&mut self, position: usize) {
        let start = char_to_byte(&self.text, position.saturating_sub(1));
        let end = self.text[start..]
            .chars()
            .next()
            .map_or(start, |c| start + c.len_utf8());
        self.error_marker = Some((error_line(&self.text, position), start..end));
    }
}

pub fn highlight_enabled(text: &str) -> bool {
    text.len() <= HIGHLIGHT_MAX_BYTES
}

/// Byte offset of the `char_idx`-th char, clamped to the text length.
pub fn char_to_byte(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map_or(text.len(), |(b, _)| b)
}

/// Adds `-- ` to each line in `lines` (0-based, end-exclusive), or removes it when all have it.
pub fn toggle_comment(text: &str, lines: Range<usize>) -> String {
    let in_range = |i: usize| lines.contains(&i);
    let all = text
        .split('\n')
        .enumerate()
        .filter(|(i, l)| in_range(*i) && !l.trim().is_empty())
        .all(|(_, l)| l.trim_start().starts_with("--"));
    text.split('\n')
        .enumerate()
        .map(|(i, l)| {
            if !in_range(i) || l.trim().is_empty() {
                l.to_string()
            } else if all {
                let ind = l.len() - l.trim_start().len();
                let rest = &l[ind + 2..];
                format!("{}{}", &l[..ind], rest.strip_prefix(' ').unwrap_or(rest))
            } else {
                format!("-- {l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 0-based line of a server error `position` (1-based, in characters; R20).
pub fn error_line(text: &str, position: usize) -> usize {
    text.chars()
        .take(position.saturating_sub(1))
        .filter(|&c| c == '\n')
        .count()
}

#[cfg(test)]
#[path = "query_tab_tests.rs"]
mod tests;
