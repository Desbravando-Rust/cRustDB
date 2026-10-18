use std::ops::Range;

use sqlparser::tokenizer::{Token, Tokenizer};

use super::{LineIndex, dialect};
use crate::model::DriverKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementSpan {
    pub range: Range<usize>,
    pub text: String,
}

fn span_of(text: &str, range: Range<usize>) -> Option<StatementSpan> {
    let s = &text[range.clone()];
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let start = range.start + (s.len() - s.trim_start().len());
    Some(StatementSpan {
        range: start..start + t.len(),
        text: t.to_string(),
    })
}

/// Splits `text` into statements. MySQL honours `DELIMITER x` lines.
pub fn split(text: &str, kind: DriverKind) -> Vec<StatementSpan> {
    let mut out = Vec::new();
    let mut delim = ";".to_string();
    let mut region_start = 0;
    let mut line_start = 0;
    let mut regions = Vec::new(); // (range, delimiter)
    for line in text.split_inclusive('\n') {
        let t = line.trim();
        let is_delim = kind == DriverKind::Mysql
            && t.len() > 9
            && t.get(..9)
                .is_some_and(|p| p.eq_ignore_ascii_case("delimiter"))
            && t[9..].starts_with(char::is_whitespace);
        if is_delim {
            regions.push((region_start..line_start, std::mem::take(&mut delim)));
            delim = t[9..].split_whitespace().next().unwrap_or(";").to_string();
            region_start = line_start + line.len();
        }
        line_start += line.len();
    }
    regions.push((region_start..text.len(), delim));

    for (range, delim) in regions {
        match split_region(text, range, &delim, kind) {
            Some(spans) => out.extend(spans),
            None => return span_of(text, 0..text.len()).into_iter().collect(),
        }
    }
    out
}

/// `None` on tokenizer error.
fn split_region(
    text: &str,
    region: Range<usize>,
    delim: &str,
    kind: DriverKind,
) -> Option<Vec<StatementSpan>> {
    let base = region.start;
    let src = &text[region];
    let tokens = Tokenizer::new(dialect(kind), src)
        .tokenize_with_location()
        .ok()?;
    let idx = LineIndex::new(src);

    let mut out = Vec::new();
    let (mut stmt_start, mut pos, mut significant) = (0, 0, false);
    for tok in &tokens {
        let (s, e) = (idx.offset(tok.span.start), idx.offset(tok.span.end));
        if s < pos {
            continue; // part of a multi-token delimiter
        }
        if matches!(tok.token, Token::Whitespace(_)) {
            continue;
        }
        // Only bare words can contain the delimiter mid-token (`END$$`).
        let hit = if src[s..].starts_with(delim) {
            Some(s)
        } else if matches!(&tok.token, Token::Word(w) if w.quote_style.is_none()) {
            src[s..e].find(delim).map(|i| s + i)
        } else {
            None
        };
        match hit {
            Some(d) => {
                if significant {
                    out.extend(span_of(text, base + stmt_start..base + d));
                }
                pos = d + delim.len();
                stmt_start = pos;
                significant = false;
            }
            None => significant = true,
        }
    }
    if significant {
        out.extend(span_of(text, base + stmt_start..base + src.len()));
    }
    Some(out)
}

/// Statement under the cursor; a cursor in the gap after a statement picks that statement.
pub fn statement_at(spans: &[StatementSpan], byte_offset: usize) -> Option<&StatementSpan> {
    let i = spans.partition_point(|s| s.range.start <= byte_offset);
    spans.get(i.checked_sub(1)?)
}

/// Statements overlapping the selection.
pub fn statements_in(spans: &[StatementSpan], selection: Range<usize>) -> Vec<&StatementSpan> {
    spans
        .iter()
        .filter(|s| s.range.start < selection.end && s.range.end > selection.start)
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn texts(s: &[StatementSpan]) -> Vec<&str> {
        s.iter().map(|x| x.text.as_str()).collect()
    }

    #[test]
    fn splits_on_semicolons() {
        let s = split("SELECT 1; SELECT 2;", DriverKind::Postgres);
        assert_eq!(texts(&s), ["SELECT 1", "SELECT 2"]);
    }
    #[test]
    fn ignores_semicolons_in_strings_and_comments() {
        let s = split("SELECT ';' -- ;\n; /* ; */ SELECT 2", DriverKind::Mysql);
        assert_eq!(s.len(), 2);
    }
    #[test]
    fn handles_pg_dollar_quotes() {
        let s = split(
            "CREATE FUNCTION f() RETURNS int AS $$ BEGIN RETURN 1; END $$ LANGUAGE plpgsql; SELECT 1",
            DriverKind::Postgres,
        );
        assert_eq!(s.len(), 2);
    }
    #[test]
    fn handles_mysql_delimiter() {
        let s = split(
            "DELIMITER $$\nCREATE PROCEDURE p() BEGIN SELECT 1; END$$\nDELIMITER ;\nSELECT 2;",
            DriverKind::Mysql,
        );
        assert_eq!(
            texts(&s),
            ["CREATE PROCEDURE p() BEGIN SELECT 1; END", "SELECT 2"]
        );
    }
    #[test]
    fn statement_at_cursor_after_semicolon_picks_previous() {
        let t = "SELECT 1;   \nSELECT 2";
        let s = split(t, DriverKind::Postgres);
        assert_eq!(statement_at(&s, 10).unwrap().text, "SELECT 1");
    }
    #[test]
    fn statement_at_with_multibyte_text() {
        let t = "SELECT 'ação';\nSELECT '🦀'";
        let s = split(t, DriverKind::Postgres);
        let off = t.find("🦀").unwrap();
        assert_eq!(statement_at(&s, off).unwrap().text, "SELECT '🦀'");
    }
    #[test]
    fn tokenizer_error_returns_whole_text() {
        let s = split("SELECT 'unterminated; SELECT 2", DriverKind::Postgres);
        assert_eq!(s.len(), 1);
    }
    #[test]
    fn range_maps_back_to_source() {
        let t = "  SELECT 1 ;";
        let s = split(t, DriverKind::Mysql);
        assert_eq!(&t[s[0].range.clone()], "SELECT 1");
    }
    #[test]
    fn statements_in_selection() {
        let t = "SELECT 1; SELECT 2; SELECT 3";
        let s = split(t, DriverKind::Postgres);
        let got: Vec<_> = statements_in(&s, 5..12)
            .iter()
            .map(|x| x.text.as_str())
            .collect();
        assert_eq!(got, ["SELECT 1", "SELECT 2"]);
    }

    #[test]
    fn mysql_multibyte_near_start_does_not_panic() {
        let s = split("SELECT 'çx'", DriverKind::Mysql);
        assert_eq!(texts(&s), ["SELECT 'çx'"]);
    }
    #[test]
    fn pg_escaped_string_with_semicolon() {
        let s = split("SELECT E'a;b'; SELECT 2", DriverKind::Postgres);
        assert_eq!(texts(&s), ["SELECT E'a;b'", "SELECT 2"]);
    }
    #[test]
    fn mysql_national_string_with_semicolon() {
        let s = split("SELECT N'a;b'; SELECT 2", DriverKind::Mysql);
        assert_eq!(texts(&s), ["SELECT N'a;b'", "SELECT 2"]);
    }
    #[test]
    fn delimiter_line_ignores_trailing_comment() {
        let s = split(
            "DELIMITER $$ -- c\nSELECT 1$$\nSELECT 2$$",
            DriverKind::Mysql,
        );
        assert_eq!(texts(&s), ["SELECT 1", "SELECT 2"]);
    }
    #[test]
    fn long_single_line_many_statements() {
        let t = "SELECT 1, 2, 3, 4; ".repeat(20000);
        let s = split(&t, DriverKind::Postgres);
        assert_eq!(s.len(), 20000);
        assert_eq!(s[19999].text, "SELECT 1, 2, 3, 4");
    }
}
