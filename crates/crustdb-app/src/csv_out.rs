use std::io::{self, BufWriter, Write};

use crustdb_core::model::{Cell, ColumnMeta, Row};

pub struct CsvOut<W: Write> {
    inner: BufWriter<W>,
}

impl<W: Write> CsvOut<W> {
    pub fn new(w: W) -> Self {
        Self {
            inner: BufWriter::new(w),
        }
    }

    pub fn header(&mut self, cols: &[ColumnMeta]) -> io::Result<()> {
        self.line(cols.iter().map(|c| field(&c.name, false, ',')))
    }

    pub fn row(&mut self, cells: &[Cell]) -> io::Result<()> {
        self.line(cells.iter().map(|c| quoted(c, ',')))
    }

    pub fn finish(mut self) -> io::Result<()> {
        self.inner.flush()
    }

    fn line(&mut self, fields: impl Iterator<Item = String>) -> io::Result<()> {
        for (i, f) in fields.enumerate() {
            if i > 0 {
                self.inner.write_all(b",")?;
            }
            self.inner.write_all(f.as_bytes())?;
        }
        self.inner.write_all(b"\n")
    }
}

/// Raw cell value: NULL is empty, binary is `0x` + hex. No quoting.
pub fn cell_text(c: &Cell) -> String {
    match c {
        Cell::Null => String::new(),
        Cell::Text { value, .. } => value.to_string(),
        Cell::Bytes { value, .. } => value.iter().fold(String::from("0x"), |mut s, b| {
            s.push_str(&format!("{b:02x}"));
            s
        }),
    }
}

/// Tab-separated rows (clipboard format), optional header line, no trailing newline.
pub fn to_tsv<'a>(
    cols: &[ColumnMeta],
    rows: impl IntoIterator<Item = &'a Row>,
    header: bool,
) -> String {
    let line = |fields: Vec<String>| fields.join("\t");
    let head = header.then(|| line(cols.iter().map(|c| field(&c.name, false, '\t')).collect()));
    let body = rows
        .into_iter()
        .map(|r| line(r.iter().map(|c| quoted(c, '\t')).collect()));
    head.into_iter().chain(body).collect::<Vec<_>>().join("\n")
}

fn quoted(c: &Cell, sep: char) -> String {
    match c {
        Cell::Text { value, .. } => field(value, true, sep),
        c => cell_text(c),
    }
}

/// Quotes on `sep` `"` `\n` `\r`; an empty string is quoted only when `quote_empty`
/// (cell values, so it differs from NULL; header names are never empty-quoted).
fn field(s: &str, quote_empty: bool, sep: char) -> String {
    if s.contains([sep, '"', '\n', '\r']) || (quote_empty && s.is_empty()) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn text(s: &str) -> Cell {
        Cell::Text {
            value: s.into(),
            truncated: false,
        }
    }

    fn out(cells: &[Cell]) -> String {
        let mut buf = Vec::new();
        let mut w = CsvOut::new(&mut buf);
        w.row(cells).unwrap();
        w.finish().unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn csv_null_is_empty_unquoted_and_empty_string_quoted() {
        assert_eq!(out(&[Cell::Null, text(""), text("a")]), ",\"\",a\n");
    }

    #[test]
    fn csv_quotes_comma_quote_newline() {
        assert_eq!(out(&[text("a,\"b")]), "\"a,\"\"b\"\n");
        assert_eq!(out(&[text("a\nb"), text("a\rb")]), "\"a\nb\",\"a\rb\"\n");
    }

    #[test]
    fn csv_binary_as_0x_hex() {
        let c = Cell::Bytes {
            value: vec![0xde, 0xad].into(),
            total_len: 2,
            truncated: false,
        };
        assert_eq!(out(&[c]), "0xdead\n");
    }

    fn col(n: &str) -> ColumnMeta {
        ColumnMeta {
            name: n.into(),
            type_name: "text".into(),
            category: crustdb_core::model::TypeCategory::Text,
            nullable: None,
            is_primary_key: false,
        }
    }

    #[test]
    fn csv_header_uses_column_names() {
        let mut buf = Vec::new();
        let mut w = CsvOut::new(&mut buf);
        w.header(&[col("id"), col("a,b")]).unwrap();
        w.finish().unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "id,\"a,b\"\n");
    }

    #[test]
    fn tsv_quotes_tab_newline_quote_and_optional_header() {
        let bytes = Cell::Bytes {
            value: vec![0xde, 0xad].into(),
            total_len: 2,
            truncated: false,
        };
        let rows: Vec<Row> = vec![
            vec![Cell::Null, text(""), text("a,b")].into(),
            vec![text("a\tb"), text("x\ny"), text("q\"")].into(),
            vec![bytes, text("r\rs"), text("plain")].into(),
        ];
        let cols = [col("id"), col("a\tb"), col("c")];
        assert_eq!(to_tsv(&cols, &rows[..1], false), "\t\"\"\ta,b");
        assert_eq!(
            to_tsv(&cols, &rows, true),
            "id\t\"a\tb\"\tc\n\
             \t\"\"\ta,b\n\
             \"a\tb\"\t\"x\ny\"\t\"q\"\"\"\n\
             0xdead\t\"r\rs\"\tplain"
        );
        assert_eq!(to_tsv(&cols, &[], true), "id\t\"a\tb\"\tc");
    }
}
