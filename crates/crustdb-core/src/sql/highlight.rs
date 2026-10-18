use std::ops::Range;

use sqlparser::keywords::Keyword;
use sqlparser::tokenizer::{Token, TokenWithSpan, Tokenizer, Whitespace};

use super::{LineIndex, dialect};
use crate::model::DriverKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Keyword,
    String,
    Number,
    Comment,
    QuotedIdent,
    Ident,
    Operator,
    Punct,
    Whitespace,
}

/// Tokens produced before any tokenizer error, plus whether the whole text tokenized.
pub(super) fn lex(text: &str, kind: DriverKind) -> (Vec<TokenWithSpan>, bool) {
    let mut buf = Vec::new();
    let ok = Tokenizer::new(dialect(kind), text)
        .tokenize_with_location_into_buf(&mut buf)
        .is_ok();
    (buf, ok)
}

fn kind_of(tok: &Token) -> TokenKind {
    match tok {
        Token::Word(w) if w.quote_style.is_some() => TokenKind::QuotedIdent,
        Token::Word(w) if w.keyword != Keyword::NoKeyword => TokenKind::Keyword,
        Token::Word(_) => TokenKind::Ident,
        Token::Number(..) => TokenKind::Number,
        Token::Whitespace(
            Whitespace::SingleLineComment { .. } | Whitespace::MultiLineComment(_),
        ) => TokenKind::Comment,
        Token::Whitespace(_) => TokenKind::Whitespace,
        Token::LParen | Token::RParen | Token::Comma | Token::SemiColon | Token::Period => {
            TokenKind::Punct
        }
        t if is_string(t) => TokenKind::String,
        _ => TokenKind::Operator,
    }
}

fn is_string(t: &Token) -> bool {
    use Token::*;
    matches!(
        t,
        SingleQuotedString(_)
            | DoubleQuotedString(_)
            | TripleSingleQuotedString(_)
            | TripleDoubleQuotedString(_)
            | DollarQuotedString(_)
            | SingleQuotedByteStringLiteral(_)
            | DoubleQuotedByteStringLiteral(_)
            | TripleSingleQuotedByteStringLiteral(_)
            | TripleDoubleQuotedByteStringLiteral(_)
            | SingleQuotedRawStringLiteral(_)
            | DoubleQuotedRawStringLiteral(_)
            | TripleSingleQuotedRawStringLiteral(_)
            | TripleDoubleQuotedRawStringLiteral(_)
            | NationalStringLiteral(_)
            | QuoteDelimitedStringLiteral(_)
            | NationalQuoteDelimitedStringLiteral(_)
            | EscapedStringLiteral(_)
            | UnicodeStringLiteral(_)
            | HexStringLiteral(_)
    )
}

/// Contiguous classified ranges covering `0..text.len()`.
pub fn highlight(text: &str, kind: DriverKind) -> Vec<(Range<usize>, TokenKind)> {
    let (tokens, ok) = lex(text, kind);
    let idx = LineIndex::new(text);
    let mut out = Vec::with_capacity(tokens.len() + 1);
    let mut pos = 0;
    for t in &tokens {
        let end = idx.offset(t.span.end).max(pos);
        if end > pos {
            out.push((pos..end, kind_of(&t.token)));
            pos = end;
        }
    }
    if pos < text.len() {
        // Unterminated literal/comment: paint the rest as a string.
        let k = if ok {
            TokenKind::Whitespace
        } else {
            TokenKind::String
        };
        out.push((pos..text.len(), k));
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const PG: DriverKind = DriverKind::Postgres;

    #[test]
    fn highlight_covers_all_bytes() {
        let t = "SELECT 'ação🦀', \"Q\" -- c\n/* x */ FROM t WHERE a >= 1.5;";
        let h = highlight(t, PG);
        let mut pos = 0;
        for (r, _) in &h {
            assert_eq!(r.start, pos);
            assert!(t.is_char_boundary(r.start) && t.is_char_boundary(r.end));
            pos = r.end;
        }
        assert_eq!(pos, t.len());
    }

    #[test]
    fn highlight_marks_keyword_string_comment() {
        let t = "SELECT 'x' -- hi\n, \"Q\", foo, 1, =";
        let h = highlight(t, PG);
        let of = |s: &str| {
            let i = t.find(s).unwrap();
            h.iter().find(|(r, _)| r.contains(&i)).unwrap().1
        };
        assert_eq!(of("SELECT"), TokenKind::Keyword);
        assert_eq!(of("'x'"), TokenKind::String);
        assert_eq!(of("-- hi"), TokenKind::Comment);
        assert_eq!(of("\"Q\""), TokenKind::QuotedIdent);
        assert_eq!(of("foo"), TokenKind::Ident);
        assert_eq!(of("1"), TokenKind::Number);
        assert_eq!(of("="), TokenKind::Operator);
        assert_eq!(of(","), TokenKind::Punct);
    }

    #[test]
    fn highlight_unterminated_string_does_not_panic() {
        for k in [PG, DriverKind::Mysql] {
            let t = "SELECT 'abc\nFROM t";
            let h = highlight(t, k);
            assert_eq!(h.last().unwrap().0.end, t.len());
            assert_eq!(h[0].1, TokenKind::Keyword);
            assert_eq!(h.last().unwrap().1, TokenKind::String);
        }
    }
}
