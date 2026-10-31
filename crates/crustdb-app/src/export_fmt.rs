//! Export formats beyond CSV: JSON Lines and SQL INSERT, picked by the file extension.
use std::io::{self, BufWriter, Write};
use std::path::Path;

use crustdb_core::dialect::Dialect;
use crustdb_core::format::hex_preview;
use crustdb_core::model::{Cell, ColumnMeta, DriverKind, TypeCategory};
use crustdb_mysql::MysqlDialect;
use crustdb_postgres::PgDialect;
use crustdb_sqlite::SqliteDialect;

use crate::csv_out::CsvOut;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Csv,
    Json,
    Sql,
}

impl Format {
    /// `.jsonl`/`.json`/`.ndjson` is JSON Lines, `.sql` is INSERT statements, `.csv` is CSV;
    /// `None` for any other extension (callers fall back to CSV).
    pub fn from_path(p: &Path) -> Option<Self> {
        let ext = p.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "csv" => Some(Self::Csv),
            "jsonl" | "json" | "ndjson" => Some(Self::Json),
            "sql" => Some(Self::Sql),
            _ => None,
        }
    }
}

pub fn dialect(kind: DriverKind) -> &'static dyn Dialect {
    match kind {
        DriverKind::Postgres => &PgDialect,
        DriverKind::Mysql => &MysqlDialect,
        DriverKind::Sqlite => &SqliteDialect,
        // unreachable: the Capabilities gate (#166) keeps Redis out of SQL paths
        DriverKind::Redis => &PgDialect,
    }
}

pub enum Out<W: Write> {
    Csv(CsvOut<W>),
    Json(BufWriter<W>, Vec<ColumnMeta>),
    /// Writer, quoted table name, driver, and the `INSERT INTO t (cols) VALUES (` prefix.
    Sql(BufWriter<W>, String, DriverKind, String),
}

impl<W: Write> Out<W> {
    /// `table` is already quoted; it is used only by the SQL format.
    pub fn new(w: W, format: Format, table: String, kind: DriverKind) -> Self {
        match format {
            Format::Csv => Self::Csv(CsvOut::new(w)),
            Format::Json => Self::Json(BufWriter::new(w), Vec::new()),
            Format::Sql => Self::Sql(BufWriter::new(w), table, kind, String::new()),
        }
    }

    pub fn header(&mut self, cols: &[ColumnMeta]) -> io::Result<()> {
        match self {
            Self::Csv(c) => c.header(cols),
            Self::Json(_, keep) => {
                *keep = cols.to_vec();
                Ok(())
            }
            Self::Sql(w, table, kind, prefix) => {
                // Literals double `'` only; with the setting off, `\'` would end the string.
                if *kind == DriverKind::Postgres {
                    writeln!(w, "SET standard_conforming_strings = on;")?;
                }
                let d = dialect(*kind);
                let names: Vec<String> = cols.iter().map(|c| d.quote_ident(&c.name)).collect();
                *prefix = format!("INSERT INTO {table} ({}) VALUES (", names.join(", "));
                Ok(())
            }
        }
    }

    pub fn row(&mut self, cells: &[Cell]) -> io::Result<()> {
        match self {
            Self::Csv(c) => c.row(cells),
            Self::Json(w, cols) => {
                // Keys written by hand so duplicate column names are all kept.
                let mut line = String::from("{");
                for (i, (col, cell)) in cols.iter().zip(cells).enumerate() {
                    if i > 0 {
                        line.push(',');
                    }
                    line.push_str(&serde_json::to_string(&col.name)?);
                    line.push(':');
                    line.push_str(&json_value(col.category, cell)?);
                }
                line.push_str("}\n");
                w.write_all(line.as_bytes())
            }
            Self::Sql(w, _, kind, prefix) => {
                let vals: Vec<String> = cells.iter().map(|c| sql_literal(*kind, c)).collect();
                writeln!(w, "{prefix}{});", vals.join(", "))
            }
        }
    }

    pub fn finish(self) -> io::Result<()> {
        match self {
            Self::Csv(c) => c.finish(),
            Self::Json(mut w, _) | Self::Sql(mut w, ..) => w.flush(),
        }
    }
}

/// Numbers stay numbers only when the text is a valid JSON number (not NaN/Infinity); the
/// original text is written so big decimals keep their precision.
fn json_value(cat: TypeCategory, c: &Cell) -> io::Result<String> {
    use TypeCategory::{Bool, Decimal, Float, Integer};
    Ok(match c {
        Cell::Null => "null".into(),
        Cell::Bytes { value, .. } => format!("\"0x{}\"", hex_preview(value, usize::MAX)),
        Cell::Text { value, .. } => match (cat, &**value) {
            (Integer | Decimal | Float, v) if v.parse::<serde_json::Number>().is_ok() => v.into(),
            (Bool, "t" | "true") => "true".into(),
            (Bool, "f" | "false") => "false".into(),
            (_, v) => serde_json::to_string(v)?,
        },
    })
}

/// A value as a SQL literal. Postgres assumes `standard_conforming_strings = on` (the dump
/// sets it); on MySQL, text with `\` or NUL goes as utf8mb4 hex so backslash escapes cannot
/// apply and JSON columns still accept it.
pub fn sql_literal(kind: DriverKind, c: &Cell) -> String {
    match (c, kind) {
        (Cell::Null, _) => "NULL".into(),
        (Cell::Text { value, .. }, DriverKind::Mysql) if value.contains(['\\', '\0']) => {
            format!("_utf8mb4 X'{}'", hex_preview(value.as_bytes(), usize::MAX))
        }
        (Cell::Text { value, .. }, _) => format!("'{}'", value.replace('\'', "''")),
        (Cell::Bytes { value, .. }, DriverKind::Postgres) => {
            format!("'\\x{}'::bytea", hex_preview(value, usize::MAX))
        }
        (Cell::Bytes { value, .. }, DriverKind::Mysql | DriverKind::Sqlite | DriverKind::Redis) => {
            format!("X'{}'", hex_preview(value, usize::MAX))
        }
    }
}
