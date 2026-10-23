//! Find/replace in the query editor (no GUI); every range is in chars.
use std::ops::Range;

use super::query_tab::char_to_byte;
use super::workspace::WorkspaceState;

#[derive(Debug, Default)]
pub struct Find {
    pub open: bool,
    pub query: String,
    pub replacement: String,
    pub case_sensitive: bool,
    /// Index into the matches; clamped when the text changes under it.
    pub current: usize,
    /// The find field takes keyboard focus on the next frame.
    pub focus: bool,
}

/// Non-overlapping occurrences of `needle` in `text`.
/// ponytail: naive O(n*m) scan, fine for editor-sized text; swap for a two-way search if it shows up.
pub fn matches(text: &str, needle: &str, case: bool) -> Vec<Range<usize>> {
    let hay: Vec<char> = text.chars().collect();
    let pat: Vec<char> = needle.chars().collect();
    let eq = |a: &char, b: &char| a == b || (!case && a.to_lowercase().eq(b.to_lowercase()));
    let mut out = vec![];
    let mut i = 0;
    while !pat.is_empty() && i + pat.len() <= hay.len() {
        if hay[i..i + pat.len()]
            .iter()
            .zip(&pat)
            .all(|(a, b)| eq(a, b))
        {
            out.push(i..i + pat.len());
            i += pat.len();
        } else {
            i += 1;
        }
    }
    out
}

impl WorkspaceState {
    /// Ctrl+F: the find bar on a query tab, the filter bar on a table tab.
    pub fn toggle_find_or_filter(&mut self) {
        if let Some(q) = self.active_query_mut() {
            q.find.open = !q.find.open;
            q.find.focus = q.find.open;
        } else if let Some(t) = self.active_table_mut() {
            t.toggle_filter_bar();
        }
    }
}

impl Find {
    fn matches(&self, text: &str) -> Vec<Range<usize>> {
        matches(text, &self.query, self.case_sensitive)
    }

    /// The current match, with `current` clamped to the matches of `text`.
    pub fn current_match(&mut self, text: &str) -> Option<Range<usize>> {
        let m = self.matches(text);
        self.current = self.current.min(m.len().saturating_sub(1));
        m.get(self.current).cloned()
    }

    /// Moves to the next (or previous) match, wrapping around.
    pub fn step(&mut self, text: &str, forward: bool) -> Option<Range<usize>> {
        let n = self.matches(text).len();
        let c = self.current.min(n.saturating_sub(1));
        self.current = match (n, forward) {
            (0, _) => 0,
            (_, true) => (c + 1) % n,
            (_, false) => (c + n - 1) % n,
        };
        self.current_match(text)
    }

    /// "2/5", or "0/0" without matches.
    pub fn count_label(&self, text: &str) -> String {
        match self.matches(text).len() {
            0 => "0/0".into(),
            n => format!("{}/{n}", self.current.min(n - 1) + 1),
        }
    }

    /// Replaces the current match; `current` then points at the next one.
    pub fn replace_current(&mut self, text: &mut String) -> bool {
        let Some(r) = self.current_match(text) else {
            return false;
        };
        let bytes = char_to_byte(text, r.start)..char_to_byte(text, r.end);
        text.replace_range(bytes, &self.replacement);
        let end = r.start + self.replacement.chars().count();
        self.current = self
            .matches(text)
            .iter()
            .position(|m| m.start >= end)
            .unwrap_or(0);
        true
    }

    /// Replaces every match and returns how many there were.
    pub fn replace_all(&mut self, text: &mut String) -> usize {
        let m = self.matches(text);
        let bytes: Vec<usize> = text
            .char_indices()
            .map(|(b, _)| b)
            .chain([text.len()])
            .collect();
        // Back to front, so earlier offsets stay valid.
        for r in m.iter().rev() {
            text.replace_range(bytes[r.start]..bytes[r.end], &self.replacement);
        }
        self.current = 0;
        m.len()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn find(q: &str) -> Find {
        Find {
            query: q.into(),
            ..Default::default()
        }
    }

    #[test]
    fn matches_case_insensitive_returns_char_ranges() {
        let text = "SELECT é, select";
        assert_eq!(matches(text, "select", false), vec![0..6, 10..16]);
        assert_eq!(matches(text, "select", true), vec![10..16]);
        assert_eq!(matches("éé x", "x", true), vec![3..4]);
        assert_eq!(matches("aaaa", "aa", true), vec![0..2, 2..4]);
        assert!(matches(text, "", false).is_empty());
    }

    #[test]
    fn next_prev_wrap_around() {
        let text = "a b a b a";
        let mut f = find("a");
        assert_eq!(f.step(text, true), Some(4..5));
        assert_eq!(f.step(text, true), Some(8..9));
        assert_eq!(f.step(text, true), Some(0..1));
        assert_eq!(f.step(text, false), Some(8..9));
        // The text lost matches: `current` is clamped.
        assert_eq!(f.current_match("a"), Some(0..1));
        assert_eq!(f.current, 0);
        assert_eq!(f.step("b", true), None);
    }

    #[test]
    fn count_label() {
        let mut f = find("a");
        assert_eq!(f.count_label("a a a a a"), "1/5");
        f.step("a a a a a", true);
        assert_eq!(f.count_label("a a a a a"), "2/5");
        assert_eq!(f.count_label("b"), "0/0");
        f.current = 9;
        assert_eq!(f.count_label("a a"), "2/2");
    }

    #[test]
    fn replace_current_replaces_one_and_keeps_position() {
        let mut text = String::from("é x é x x");
        let mut f = find("x");
        f.replacement = "yy".into();
        f.current = 1;
        assert!(f.replace_current(&mut text));
        assert_eq!(text, "é x é yy x");
        assert_eq!(f.current_match(&text), Some(9..10));
        // Replacing the last one wraps to the first.
        assert!(f.replace_current(&mut text));
        assert_eq!(text, "é x é yy yy");
        assert_eq!(f.current_match(&text), Some(2..3));
        assert!(!find("z").replace_current(&mut text));
        assert_eq!(text, "é x é yy yy");
    }

    #[test]
    fn replace_all_returns_count() {
        let mut text = String::from("a é A");
        let mut f = find("a");
        f.replacement = "aa".into();
        assert_eq!(f.replace_all(&mut text), 2);
        assert_eq!(text, "aa é aa");
        assert_eq!(find("z").replace_all(&mut text), 0);
        assert_eq!(text, "aa é aa");
    }
}
