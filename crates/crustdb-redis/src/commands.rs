//! Pure command construction: argument vectors and binary-safe `Cmd`s.

use redis::Cmd;

/// Every argument, command name included, is sent as raw bytes.
pub(crate) fn build<I, A>(argv: I) -> Cmd
where
    I: IntoIterator<Item = A>,
    A: AsRef<[u8]>,
{
    let mut cmd = Cmd::new();
    for a in argv {
        cmd.arg(a.as_ref());
    }
    cmd
}

/// `SCAN`/`HSCAN`/`SSCAN`: `key` is `None` for the keyspace-wide `SCAN`.
pub(crate) fn scan_argv(
    name: &str,
    key: Option<&[u8]>,
    cursor: &str,
    pattern: Option<&str>,
    count: usize,
) -> Vec<Vec<u8>> {
    let mut v = vec![name.as_bytes().to_vec()];
    v.extend(key.map(<[u8]>::to_vec));
    v.push(cursor.as_bytes().to_vec());
    if let Some(p) = pattern {
        v.extend([b"MATCH".to_vec(), p.as_bytes().to_vec()]);
    }
    v.extend([b"COUNT".to_vec(), count.to_string().into_bytes()]);
    v
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn scan_matches_only_when_a_pattern_is_given() {
        // Act
        let all = scan_argv("SCAN", None, "0", None, 100);
        let some = scan_argv("HSCAN", Some(b"h"), "7", Some("a*"), 5);

        // Assert
        assert_eq!(all, [&b"SCAN"[..], b"0", b"COUNT", b"100"]);
        assert_eq!(
            some,
            [&b"HSCAN"[..], b"h", b"7", b"MATCH", b"a*", b"COUNT", b"5"]
        );
    }

    #[test]
    fn build_sends_name_and_binary_args_verbatim() {
        // Arrange
        let argv: [&[u8]; 3] = [b"get", b"\xff\x00key", b"x y"];

        // Act
        let packed = build(argv).get_packed_command();

        // Assert
        let expected = b"*3\r\n$3\r\nget\r\n$5\r\n\xff\x00key\r\n$3\r\nx y\r\n";
        assert_eq!(packed, expected);
    }
}
