use std::io::{self, BufWriter, Write};

use crustdb_core::model::{Cell, ColumnMeta};

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
        self.line(cols.iter().map(|c| field(&c.name, false)))
    }

    pub fn row(&mut self, cells: &[Cell]) -> io::Result<()> {
        self.line(cells.iter().map(|c| match c {
            Cell::Null => String::new(),
            Cell::Text { value, .. } => field(value, true),
            Cell::Bytes { value, .. } => value.iter().fold(String::from("0x"), |mut s, b| {
                s.push_str(&format!("{b:02x}"));
                s
            }),
        }))
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

/// Quotes on `,` `"` `\n` `\r`; an empty string is quoted only when `quote_empty`
/// (cell values, so it differs from NULL; header names are never empty-quoted).
fn field(s: &str, quote_empty: bool) -> String {
    if s.contains([',', '"', '\n', '\r']) || (quote_empty && s.is_empty()) {
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

    #[test]
    fn csv_header_uses_column_names() {
        let col = |n: &str| ColumnMeta {
            name: n.into(),
            type_name: "text".into(),
            category: crustdb_core::model::TypeCategory::Text,
            nullable: None,
            is_primary_key: false,
        };
        let mut buf = Vec::new();
        let mut w = CsvOut::new(&mut buf);
        w.header(&[col("id"), col("a,b")]).unwrap();
        w.finish().unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "id,\"a,b\"\n");
    }
}
