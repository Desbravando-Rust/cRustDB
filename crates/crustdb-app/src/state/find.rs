//! Find/replace in the query editor (no GUI); every range is in chars.
use std::ops::Range;

#[derive(Debug, Default)]
pub struct Find {
    pub open: bool,
    pub query: String,
    pub replacement: String,
    pub case_sensitive: bool,
    /// Index into the matches; clamped when the text changes under it.
    pub current: usize,
}

/// Non-overlapping occurrences of `needle` in `text`.
pub fn matches(_text: &str, _needle: &str, _case: bool) -> Vec<Range<usize>> {
    vec![]
}

impl Find {
    /// The current match, with `current` clamped to the matches of `text`.
    pub fn current_match(&mut self, _text: &str) -> Option<Range<usize>> {
        None
    }

    /// Moves to the next (or previous) match, wrapping around.
    pub fn step(&mut self, _text: &str, _forward: bool) -> Option<Range<usize>> {
        None
    }

    /// "2/5", or "0/0" without matches.
    pub fn count_label(&self, _text: &str) -> String {
        String::new()
    }

    /// Replaces the current match; `current` then points at the next one.
    pub fn replace_current(&mut self, _text: &mut String) -> bool {
        false
    }

    /// Replaces every match and returns how many there were.
    pub fn replace_all(&mut self, _text: &mut String) -> usize {
        0
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
