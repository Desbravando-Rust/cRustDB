use std::ops::Range;

use sqlparser::keywords::Keyword;
use sqlparser::tokenizer::Token;

use super::{LineIndex, highlight::lex, split, statement_at};
use crate::model::DriverKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expect {
    Keyword,
    Table,
    Column { qualifier: Option<String> },
    Any,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableAlias {
    pub name: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionContext {
    pub prefix: String,
    pub prefix_range: Range<usize>,
    pub expect: Expect,
    pub referenced_tables: Vec<TableAlias>,
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

fn kw(t: &Token) -> Option<Keyword> {
    match t {
        Token::Word(w) if w.quote_style.is_none() => Some(w.keyword),
        _ => None,
    }
}

fn word(t: &Token) -> Option<&str> {
    match t {
        Token::Word(w) => Some(&w.value),
        _ => None,
    }
}

pub fn completion_context(text: &str, byte_offset: usize, kind: DriverKind) -> CompletionContext {
    let mut off = byte_offset.min(text.len());
    while !text.is_char_boundary(off) {
        off -= 1;
    }
    let start = text[..off]
        .char_indices()
        .rev()
        .take_while(|&(_, c)| is_ident_char(c))
        .last()
        .map_or(off, |(i, _)| i);
    let mut ctx = CompletionContext {
        prefix: text[start..off].to_string(),
        prefix_range: start..off,
        expect: Expect::Keyword,
        referenced_tables: Vec::new(),
    };

    let spans = split(text, kind);
    let Some(stmt) = statement_at(&spans, off) else {
        return ctx;
    };
    // Cursor past the statement's delimiter: a new, empty statement.
    if start >= stmt.range.end && text[stmt.range.end..start].contains(|c: char| !c.is_whitespace())
    {
        return ctx;
    }
    let (tokens, _) = lex(&stmt.text, kind);
    let idx = LineIndex::new(&stmt.text);
    let sig: Vec<(&Token, usize)> = tokens
        .iter()
        .filter(|t| !matches!(t.token, Token::Whitespace(_)))
        .map(|t| (&t.token, stmt.range.start + idx.offset(t.span.end)))
        .collect();
    // Tokens that end at or before the prefix; the cursor may sit past the statement end.
    let before = sig.partition_point(|&(_, end)| end <= start);
    let before: Vec<&Token> = sig[..before].iter().map(|&(t, _)| t).collect();
    ctx.expect = expect_at(&before);
    let all: Vec<&Token> = sig.iter().map(|&(t, _)| t).collect();
    ctx.referenced_tables = referenced_tables(&all);
    ctx
}

fn expect_at(before: &[&Token]) -> Expect {
    let Some((last, rest)) = before.split_last() else {
        return Expect::Keyword;
    };
    if matches!(last, Token::Period) {
        return match rest.last().and_then(|t| word(t)) {
            Some(q) => Expect::Column {
                qualifier: Some(q.to_string()),
            },
            None => Expect::Any,
        };
    }
    use Keyword::*;
    if matches!(kw(last), Some(FROM | JOIN | INTO | UPDATE | TABLE)) {
        return Expect::Table;
    }
    // ponytail: nearest clause keyword wins; parentheses/subqueries are not tracked.
    let clause = before.iter().rev().find_map(|t| {
        kw(t).filter(|k| {
            matches!(
                k,
                SELECT
                    | WHERE
                    | ON
                    | BY
                    | HAVING
                    | SET
                    | AND
                    | OR
                    | FROM
                    | JOIN
                    | INTO
                    | UPDATE
                    | TABLE
                    | LIMIT
                    | OFFSET
                    | VALUES
            )
        })
    });
    match clause {
        Some(SELECT | WHERE | ON | BY | HAVING | SET | AND | OR) => {
            Expect::Column { qualifier: None }
        }
        _ => Expect::Any,
    }
}

fn referenced_tables(toks: &[&Token]) -> Vec<TableAlias> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let list = matches!(kw(toks[i]), Some(Keyword::FROM));
        if matches!(
            kw(toks[i]),
            Some(Keyword::FROM | Keyword::JOIN | Keyword::INTO | Keyword::UPDATE)
        ) {
            i += 1;
            while let Some(mut name) = toks.get(i).and_then(|t| word(t)).map(str::to_string) {
                i += 1;
                while matches!(toks.get(i), Some(Token::Period)) {
                    let Some(part) = toks.get(i + 1).and_then(|t| word(t)) else {
                        break;
                    };
                    name = format!("{name}.{part}");
                    i += 2;
                }
                if matches!(toks.get(i).and_then(|t| kw(t)), Some(Keyword::AS)) {
                    i += 1;
                }
                let alias = match toks.get(i) {
                    Some(Token::Word(w))
                        if w.quote_style.is_some() || w.keyword == Keyword::NoKeyword =>
                    {
                        i += 1;
                        Some(w.value.clone())
                    }
                    _ => None,
                };
                out.push(TableAlias { name, alias });
                if list && matches!(toks.get(i), Some(Token::Comma)) {
                    i += 1;
                } else {
                    break;
                }
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Indices into `candidates`: case-insensitive prefix matches, then substring matches.
pub fn rank(candidates: &[&str], prefix: &str, max: usize) -> Vec<usize> {
    let p = prefix.to_lowercase();
    let lowered: Vec<String> = candidates.iter().map(|c| c.to_lowercase()).collect();
    let starts = (0..candidates.len()).filter(|&i| lowered[i].starts_with(&p));
    let contains =
        (0..candidates.len()).filter(|&i| !lowered[i].starts_with(&p) && lowered[i].contains(&p));
    starts.chain(contains).take(max).collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const PG: DriverKind = DriverKind::Postgres;

    fn ctx(text: &str) -> CompletionContext {
        let off = text.find('|').unwrap();
        completion_context(&text.replace('|', ""), off, PG)
    }

    #[test]
    fn after_from_expects_table() {
        // Act
        let c = ctx("SELECT * FROM us|");

        // Assert
        assert_eq!(c.expect, Expect::Table);
        assert_eq!(c.prefix, "us");
        assert_eq!(c.prefix_range, 14..16);
    }

    #[test]
    fn after_alias_dot_expects_column_with_qualifier() {
        // Act
        let c = ctx("SELECT u.| FROM users u");

        // Assert
        assert_eq!(
            c.expect,
            Expect::Column {
                qualifier: Some("u".into())
            }
        );
        assert_eq!(c.prefix, "");
        assert_eq!(
            c.referenced_tables,
            [TableAlias {
                name: "users".into(),
                alias: Some("u".into())
            }]
        );
    }

    #[test]
    fn select_list_expects_column_without_qualifier() {
        // Act & Assert
        assert_eq!(ctx("SELECT na|").expect, Expect::Column { qualifier: None });
        assert_eq!(
            ctx("SELECT * FROM t WHERE a = 1 AND b|").expect,
            Expect::Column { qualifier: None }
        );
    }

    #[test]
    fn statement_start_expects_keyword_and_empty_text_is_empty() {
        // Act & Assert
        assert_eq!(ctx("SEL|").expect, Expect::Keyword);
        assert_eq!(ctx("SELECT 1; UP|").expect, Expect::Keyword);
        let c = completion_context("", 0, PG);
        assert_eq!(c.expect, Expect::Keyword);
        assert!(c.referenced_tables.is_empty());
    }

    #[test]
    fn referenced_tables_handles_joins_as_and_lists() {
        // Act
        let c = ctx("SELECT | FROM a AS x, c JOIN s.b y ON x.i = y.i WHERE 1");

        // Assert
        let t: Vec<_> = c
            .referenced_tables
            .iter()
            .map(|t| (t.name.as_str(), t.alias.as_deref()))
            .collect();
        assert_eq!(t, [("a", Some("x")), ("c", None), ("s.b", Some("y"))]);
    }

    #[test]
    fn clause_keywords_only_decide_expect() {
        // Arrange
        let col = Expect::Column { qualifier: None };

        // Act & Assert
        for t in [
            "SELECT * FROM t ORDER BY |",
            "SELECT * FROM t GROUP BY |",
            "SELECT * FROM t GROUP BY a HAVING |",
            "UPDATE t SET |",
            "SELECT * FROM a JOIN b ON |",
            "SELECT name, |",
            "SELECT * FROM t WHERE x IN (|",
        ] {
            assert_eq!(ctx(t).expect, col, "{t}");
        }
        assert_eq!(ctx("SELECT * FROM t LIMIT |").expect, Expect::Any);
    }

    #[test]
    fn cursor_after_semicolon_starts_new_statement() {
        // Act & Assert
        for t in ["SELECT 1; |", "SELECT * FROM t;\n|"] {
            let c = ctx(t);
            assert_eq!(c.expect, Expect::Keyword, "{t}");
            assert!(c.referenced_tables.is_empty(), "{t}");
        }
    }

    #[test]
    fn multibyte_prefix_range() {
        // Act
        let c = ctx("SELECT ação|");

        // Assert
        assert_eq!(c.prefix, "ação");
        assert_eq!(c.prefix_range, 7..13);
    }

    #[test]
    fn rank_prefix_before_substring() {
        // Act & Assert
        assert_eq!(
            rank(&["user_id", "id", "users", "my_user"], "us", 50),
            [0, 2, 3]
        );
        assert_eq!(rank(&["Users"], "US", 50), [0]);
    }

    #[test]
    fn rank_caps_at_max() {
        // Arrange
        let owned: Vec<String> = (0..100).map(|i| format!("c{i}")).collect();
        let c: Vec<&str> = owned.iter().map(String::as_str).collect();

        // Act & Assert
        assert_eq!(rank(&c, "c", 50).len(), 50);
    }
}
