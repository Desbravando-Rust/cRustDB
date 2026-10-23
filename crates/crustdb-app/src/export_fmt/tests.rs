#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::path::Path;

use crustdb_core::model::{Cell, ColumnMeta, DriverKind, TypeCategory};

use super::*;

fn text(s: &str) -> Cell {
    Cell::Text {
        value: s.into(),
        truncated: false,
    }
}

fn bytes(b: &[u8]) -> Cell {
    Cell::Bytes {
        value: b.into(),
        total_len: b.len() as u64,
        truncated: false,
    }
}

fn col(name: &str, category: TypeCategory) -> ColumnMeta {
    ColumnMeta {
        name: name.into(),
        type_name: "t".into(),
        category,
        nullable: None,
        is_primary_key: false,
    }
}

fn write(format: Format, kind: DriverKind, cols: &[ColumnMeta], rows: &[Vec<Cell>]) -> String {
    let mut buf = Vec::new();
    let mut w = Out::new(&mut buf, format, "\"t\"".into(), kind);
    w.header(cols).unwrap();
    for r in rows {
        w.row(r).unwrap();
    }
    w.finish().unwrap();
    String::from_utf8(buf).unwrap()
}

#[test]
fn format_from_extension() {
    let f = |p: &str| Format::from_path(Path::new(p));
    assert_eq!(f("a.jsonl"), Format::Json);
    assert_eq!(f("a.SQL"), Format::Sql);
    assert_eq!(f("a.csv"), Format::Csv);
    assert_eq!(f("a"), Format::Csv);
    assert_eq!(f("a.txt"), Format::Csv);
}

#[test]
fn jsonl_null_number_text_bytes() {
    use TypeCategory::*;
    let cols = [
        col("i", Integer),
        col("d", Decimal),
        col("f", Float),
        col("nan", Float),
        col("s", Text),
        col("n", Text),
        col("b", Binary),
        col("ok", Bool),
    ];
    let row = vec![
        text("-12"),
        text("1.50"),
        text("2.5e3"),
        text("NaN"),
        text("7"),
        Cell::Null,
        bytes(&[0xde, 0xad]),
        text("t"),
    ];
    assert_eq!(
        write(Format::Json, DriverKind::Postgres, &cols, &[row]),
        "{\"i\":-12,\"d\":1.50,\"f\":2.5e3,\"nan\":\"NaN\",\"s\":\"7\",\"n\":null,\"b\":\"0xdead\",\"ok\":true}\n"
    );
}

#[test]
fn jsonl_escapes_quotes_newlines_in_keys_and_values() {
    let cols = [
        col("a\"b", TypeCategory::Text),
        col("a\"b", TypeCategory::Text),
    ];
    let rows = [
        vec![text("x\ny\\\""), text("")],
        vec![Cell::Null, text("z")],
    ];
    assert_eq!(
        write(Format::Json, DriverKind::Mysql, &cols, &rows),
        "{\"a\\\"b\":\"x\\ny\\\\\\\"\",\"a\\\"b\":\"\"}\n{\"a\\\"b\":null,\"a\\\"b\":\"z\"}\n"
    );
}

#[test]
fn sql_literal_pg_doubles_quote_and_bytea_hex() {
    let pg = DriverKind::Postgres;
    assert_eq!(sql_literal(pg, &text("it's a\\b")), "'it''s a\\b'");
    assert_eq!(sql_literal(pg, &bytes(&[0x00, 0xff])), "'\\x00ff'::bytea");
}

#[test]
fn sql_literal_mysql_backslash_goes_hex() {
    let my = DriverKind::Mysql;
    assert_eq!(sql_literal(my, &text("a\\b")), "X'615c62'");
    assert_eq!(sql_literal(my, &text("a\0")), "X'6100'");
    assert_eq!(sql_literal(my, &text("it's")), "'it''s'");
    assert_eq!(sql_literal(my, &bytes(&[0xab])), "X'ab'");
}

#[test]
fn sql_literal_null() {
    assert_eq!(sql_literal(DriverKind::Postgres, &Cell::Null), "NULL");
    assert_eq!(sql_literal(DriverKind::Mysql, &Cell::Null), "NULL");
}

#[test]
fn sql_insert_quotes_idents_and_table() {
    let cols = [
        col("id", TypeCategory::Integer),
        col("we\"ird", TypeCategory::Text),
    ];
    let rows = [vec![text("1"), Cell::Null], vec![text("2"), text("x")]];
    assert_eq!(
        write(Format::Sql, DriverKind::Postgres, &cols, &rows),
        "INSERT INTO \"t\" (\"id\", \"we\"\"ird\") VALUES ('1', NULL);\n\
         INSERT INTO \"t\" (\"id\", \"we\"\"ird\") VALUES ('2', 'x');\n"
    );
    let cols = [col("a`b", TypeCategory::Text)];
    assert_eq!(
        write(Format::Sql, DriverKind::Mysql, &cols, &[vec![text("v")]]),
        "INSERT INTO \"t\" (`a``b`) VALUES ('v');\n"
    );
}
