use sqlparser::keywords::Keyword;
use sqlparser::tokenizer::{Token, Whitespace};

use super::LineIndex;
use super::highlight::lex;
use crate::model::DriverKind;

/// A non-blank token: its source text, the token, and whether blanks preceded it.
struct Item<'a> {
    src: &'a str,
    tok: &'a Token,
    spaced: bool,
}

/// Re-lays out `text` with one clause per line, copying every token verbatim from the source
/// (keyword case, literals and comments untouched). `None` when the text does not tokenize.
pub fn beautify(text: &str, kind: DriverKind) -> Option<String> {
    let (tokens, ok) = lex(text, kind);
    if !ok {
        return None;
    }
    let idx = LineIndex::new(text);
    let mut items = Vec::with_capacity(tokens.len());
    let (mut pos, mut spaced) = (0, false);
    // Spans must tile the text: MySQL comment hints (`/*!40101 ... */`) are re-tokenized with
    // shifted spans, and copying those would drop characters.
    for t in &tokens {
        if idx.offset(t.span.start) != pos {
            return None;
        }
        let end = idx.offset(t.span.end).max(pos);
        let src = &text[pos..end];
        pos = end;
        if let Token::Whitespace(Whitespace::Space | Whitespace::Newline | Whitespace::Tab) =
            t.token
        {
            if !src.trim().is_empty() {
                return None;
            }
            spaced = true;
            continue;
        }
        items.push(Item {
            src: src.trim(),
            tok: &t.token,
            spaced,
        });
        spaced = false;
    }
    if pos != text.len() {
        return None;
    }

    let mut out = String::with_capacity(text.len());
    let mut depth = 0usize;
    // Per paren level: clause breaks apply (top level or a subquery).
    let mut query = vec![true];
    for (i, it) in items.iter().enumerate() {
        if matches!(it.tok, Token::RParen) {
            depth = depth.saturating_sub(1);
            if query.len() > 1 {
                query.pop();
            }
        }
        let newline = |out: &mut String| {
            out.push('\n');
            out.push_str(&"  ".repeat(depth));
        };
        match i.checked_sub(1).map(|j| items[j].tok) {
            None => {}
            Some(Token::SemiColon) => out.push_str("\n\n"),
            Some(p) if is_line_comment(p) => newline(&mut out),
            Some(Token::LParen | Token::Period | Token::DoubleColon) => {}
            _ if query.last() == Some(&true) && breaks(&items, i) => newline(&mut out),
            _ if matches!(
                it.tok,
                Token::Comma
                    | Token::SemiColon
                    | Token::RParen
                    | Token::Period
                    | Token::DoubleColon
            ) => {}
            _ if matches!(it.tok, Token::LParen) && !it.spaced => {}
            _ => out.push(' '),
        }
        out.push_str(it.src);
        if matches!(it.tok, Token::LParen) {
            depth += 1;
            let sub = items
                .get(i + 1)
                .is_some_and(|n| matches!(keyword(n.tok), Keyword::SELECT | Keyword::WITH));
            query.push(sub);
        }
    }
    Some(out)
}

fn is_line_comment(t: &Token) -> bool {
    matches!(t, Token::Whitespace(Whitespace::SingleLineComment { .. }))
}

fn keyword(t: &Token) -> Keyword {
    match t {
        Token::Word(w) if w.quote_style.is_none() => w.keyword,
        _ => Keyword::NoKeyword,
    }
}

/// Whether a clause starts at `items[i]` (multi-word clauses break before their first word).
fn breaks(items: &[Item<'_>], i: usize) -> bool {
    use Keyword::*;
    let kw = |j: usize| items.get(j).map_or(NoKeyword, |it| keyword(it.tok));
    let join_mod = |k: Keyword| matches!(k, LEFT | RIGHT | INNER | FULL | CROSS | NATURAL | OUTER);
    let prev = i.checked_sub(1).map_or(NoKeyword, kw);
    match kw(i) {
        SELECT | FROM | WHERE | HAVING | LIMIT | UNION | VALUES | SET | RETURNING => true,
        GROUP | ORDER => kw(i + 1) == BY,
        JOIN => !join_mod(prev),
        OUTER => false,
        k if join_mod(k) => !join_mod(prev) && (i + 1..i + 4).any(|j| kw(j) == JOIN),
        _ => false,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const PG: DriverKind = DriverKind::Postgres;
    const MY: DriverKind = DriverKind::Mysql;

    #[test]
    fn beautify_breaks_major_clauses() {
        // Act & Assert
        assert_eq!(
            beautify("select a,b from t where x=1", PG).unwrap(),
            "select a, b\nfrom t\nwhere x = 1"
        );
        assert_eq!(
            beautify(
                "SELECT a FROM t LEFT OUTER JOIN u ON t.id=u.id GROUP BY a ORDER BY a",
                PG
            )
            .unwrap(),
            "SELECT a\nFROM t\nLEFT OUTER JOIN u ON t.id = u.id\nGROUP BY a\nORDER BY a"
        );
        assert_eq!(
            beautify("select left(s, 2) from t where a in (select b from u)", PG).unwrap(),
            "select left(s, 2)\nfrom t\nwhere a in (select b\n  from u)"
        );
    }

    #[test]
    fn beautify_preserves_literals_and_comments() {
        // Act & Assert
        let pg = beautify("select 'a  ;  b', $$ x  y $$ -- c  c\nfrom /* c  */ t", PG).unwrap();
        assert_eq!(pg, "select 'a  ;  b', $$ x  y $$ -- c  c\nfrom /* c  */ t");
        let my = beautify("select `col  x` from t -- c", MY).unwrap();
        assert_eq!(my, "select `col  x`\nfrom t -- c");
    }

    #[test]
    fn beautify_is_idempotent() {
        // Act & Assert
        for (t, k) in [
            ("select a,b from t where x=1", PG),
            (
                "select count(*)::int from t -- c\n where a in (select b from u) limit 5",
                PG,
            ),
            (
                "insert into t (a, b) values (1, 'x') returning a; update t set a=1",
                PG,
            ),
            ("select `a` from t # c\n order by `a`", MY),
        ] {
            let once = beautify(t, k).unwrap();
            assert_eq!(beautify(&once, k).unwrap(), once, "{t}");
        }
    }

    #[test]
    fn beautify_returns_none_on_tokenize_error() {
        // Act & Assert
        assert_eq!(beautify("select 'abc from t", PG), None);
        assert_eq!(beautify("select 'abc from t", MY), None);
    }

    #[test]
    fn beautify_never_drops_chars_of_mysql_comment_hints() {
        // Arrange
        let t = "/*!40101 SET NAMES utf8 */; select 1";

        // Act & Assert
        assert_eq!(beautify(t, MY), None);
    }

    #[test]
    fn beautify_keeps_statement_separation() {
        // Act & Assert
        assert_eq!(
            beautify("select 1;select 2;", PG).unwrap(),
            "select 1;\n\nselect 2;"
        );
    }
}
