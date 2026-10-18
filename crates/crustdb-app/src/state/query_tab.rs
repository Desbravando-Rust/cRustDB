//! Query editor tab: text, run targets and error position mapping (no GUI).
use std::ops::Range;
use std::path::PathBuf;

use crustdb_core::model::DriverKind;
use crustdb_core::sql::{split, statement_at};

use crate::runtime::{RequestId, TabId, TxState};

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
        }
    }

    /// Statements to evaluate: all, the selection's, or the one under the cursor.
    pub fn targets(
        &self,
        cursor_byte: usize,
        selection: Option<Range<usize>>,
        all: bool,
    ) -> Vec<String> {
        // A selection runs exactly the selected text, never statements that merely touch it.
        let selection = selection.filter(|r| !r.is_empty());
        let spans = match (&selection, all) {
            (Some(r), false) => split(&self.text[r.clone()], self.driver),
            _ => split(&self.text, self.driver),
        };
        if all || selection.is_some() {
            return spans.into_iter().map(|s| s.text).collect();
        }
        statement_at(&spans, cursor_byte)
            .map(|s| vec![s.text.clone()])
            .unwrap_or_default()
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
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
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
}
