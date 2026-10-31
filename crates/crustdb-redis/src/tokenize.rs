//! `redis-cli` style line splitting (semantics of `sdssplitargs`).

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TokenizeError {
    #[error("unbalanced quotes in command line")]
    UnbalancedQuotes,
    #[error("closing quote must be followed by a space")]
    QuoteNotFollowedBySpace,
}

fn hex(b: u8) -> Option<u8> {
    (b as char).to_digit(16).map(|d| d as u8)
}

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\n' | b'\r' | b'\t' | 0x0b | 0x0c)
}

/// Splits a command line into raw arguments; each one is meant to be sent as its own bulk string.
pub fn tokenize(line: &str) -> Result<Vec<Vec<u8>>, TokenizeError> {
    let b = line.as_bytes();
    let mut args = Vec::new();
    let mut i = 0;
    loop {
        while i < b.len() && is_space(b[i]) {
            i += 1;
        }
        if i >= b.len() {
            return Ok(args);
        }
        let mut cur = Vec::new();
        let quote = matches!(b[i], b'"' | b'\'').then(|| b[i]);
        if quote.is_some() {
            i += 1;
        }
        loop {
            let Some(&c) = b.get(i) else {
                if quote.is_some() {
                    return Err(TokenizeError::UnbalancedQuotes);
                }
                break;
            };
            match quote {
                Some(q) if c == q => {
                    i += 1;
                    if b.get(i).is_some_and(|&n| !is_space(n)) {
                        return Err(TokenizeError::QuoteNotFollowedBySpace);
                    }
                    break;
                }
                Some(b'"') if c == b'\\' && i + 1 < b.len() => {
                    let n = b[i + 1];
                    let hexed = (n == b'x')
                        .then(|| Some(hex(*b.get(i + 2)?)? << 4 | hex(*b.get(i + 3)?)?))
                        .flatten();
                    if let Some(v) = hexed {
                        cur.push(v);
                        i += 4;
                    } else {
                        cur.push(match n {
                            b'n' => b'\n',
                            b'r' => b'\r',
                            b't' => b'\t',
                            b'b' => 0x08,
                            b'a' => 0x07,
                            other => other,
                        });
                        i += 2;
                    }
                }
                Some(b'\'') if c == b'\\' && b.get(i + 1) == Some(&b'\'') => {
                    cur.push(b'\'');
                    i += 2;
                }
                None if is_space(c) => break,
                _ => {
                    cur.push(c);
                    i += 1;
                }
            }
        }
        args.push(cur);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn tokenizer_keeps_quoted_crlf_as_single_arg() {
        // Act
        let args = tokenize(r#"SET k "v\r\nFLUSHALL""#).unwrap();

        // Assert
        assert_eq!(
            args,
            vec![b"SET".to_vec(), b"k".to_vec(), b"v\r\nFLUSHALL".to_vec()]
        );
    }

    #[test]
    fn unterminated_quote_errors() {
        // Act & Assert
        assert_eq!(
            tokenize(r#"GET "abc"#),
            Err(TokenizeError::UnbalancedQuotes)
        );
        assert_eq!(tokenize("GET 'abc"), Err(TokenizeError::UnbalancedQuotes));
    }

    #[test]
    fn hex_escape_yields_raw_byte() {
        // Act
        let args = tokenize(r#"GET "\xff\x00a""#).unwrap();

        // Assert
        assert_eq!(args, vec![b"GET".to_vec(), vec![0xff, 0x00, b'a']]);
    }

    #[test]
    fn closing_quote_must_be_followed_by_space() {
        // Act & Assert
        assert_eq!(
            tokenize(r#"GET "a"b"#),
            Err(TokenizeError::QuoteNotFollowedBySpace)
        );
    }

    #[test]
    fn single_quotes_only_unescape_quote() {
        // Act
        let args = tokenize(r"SET k 'a\'b\n'").unwrap();

        // Assert
        assert_eq!(args[2], b"a'b\\n".to_vec());
    }
}
