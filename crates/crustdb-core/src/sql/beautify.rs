use crate::model::DriverKind;

/// Re-lays out `text` with one clause per line. `None` when the text does not tokenize.
pub fn beautify(text: &str, kind: DriverKind) -> Option<String> {
    let _ = (text, kind);
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const PG: DriverKind = DriverKind::Postgres;
    const MY: DriverKind = DriverKind::Mysql;

    #[test]
    fn beautify_breaks_major_clauses() {
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
        let pg = beautify("select 'a  ;  b', $$ x  y $$ -- c  c\nfrom /* c  */ t", PG).unwrap();
        assert_eq!(pg, "select 'a  ;  b', $$ x  y $$ -- c  c\nfrom /* c  */ t");
        let my = beautify("select `col  x` from t -- c", MY).unwrap();
        assert_eq!(my, "select `col  x`\nfrom t -- c\n");
    }

    #[test]
    fn beautify_is_idempotent() {
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
        assert_eq!(beautify("select 'abc from t", PG), None);
        assert_eq!(beautify("select 'abc from t", MY), None);
    }

    #[test]
    fn beautify_keeps_statement_separation() {
        assert_eq!(
            beautify("select 1;select 2;", PG).unwrap(),
            "select 1;\n\nselect 2;"
        );
    }
}
