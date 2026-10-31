#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::model::{DriverKind, TypeCategory};

struct TestDialect;
impl Dialect for TestDialect {
    fn kind(&self) -> DriverKind {
        DriverKind::Postgres
    }
    fn quote_ident(&self, s: &str) -> String {
        format!("\"{}\"", s.replace('"', "\"\""))
    }
    fn qualified_name(&self, o: &ObjectRef) -> String {
        match &o.schema {
            Some(s) => format!("{}.{}", self.quote_ident(s), self.quote_ident(&o.name)),
            None => self.quote_ident(&o.name),
        }
    }
    fn param(&self, index: usize, _: &ColumnMeta) -> String {
        format!("${index}")
    }
    fn select_expr(&self, c: &ColumnMeta) -> String {
        self.quote_ident(&c.name)
    }
    fn ci_like(&self, c: &ColumnMeta, p: &str) -> String {
        format!("{} ILIKE {p} ESCAPE '!'", self.as_text(c))
    }
    fn as_text(&self, c: &ColumnMeta) -> String {
        format!("{}::text", self.quote_ident(&c.name))
    }
}

fn col(name: &str, pk: bool) -> ColumnMeta {
    ColumnMeta {
        name: name.into(),
        type_name: "t".into(),
        category: TypeCategory::Text,
        nullable: None,
        is_primary_key: pk,
    }
}

fn obj(kind: ObjectKind) -> ObjectRef {
    ObjectRef {
        database: "db".into(),
        schema: Some("public".into()),
        name: "t".into(),
        kind,
    }
}

fn text(s: &str) -> Cell {
    Cell::Text {
        value: s.into(),
        truncated: false,
    }
}

fn cols() -> Vec<ColumnMeta> {
    vec![col("id", true), col("c", false), col("k", true)]
}

fn row() -> Vec<Cell> {
    vec![text("1"), text("old"), text("a")]
}

fn build(row: &[Cell], column: usize, v: Option<&str>) -> Result<BuiltQuery, QueryError> {
    build_update(
        &TestDialect,
        &obj(ObjectKind::Table),
        &cols(),
        row,
        &[(column, v.map(String::from))],
    )
}

#[test]
fn update_binds_value_and_all_pk_columns() {
    // Act
    let b = build(&row(), 1, Some("new")).unwrap();

    // Assert
    assert_eq!(
        b.sql,
        r#"UPDATE "public"."t" SET "c" = $1 WHERE "id" = $2 AND "k" = $3"#
    );
    assert_eq!(
        b.params,
        vec![
            Param(Some("new".into())),
            Param(Some("1".into())),
            Param(Some("a".into()))
        ]
    );
}

#[test]
fn update_null_value_binds_none() {
    // Act
    let b = build(&row(), 1, None).unwrap();

    // Assert
    assert_eq!(b.params[0], Param(None));
}

#[test]
fn rejects_table_without_pk() {
    // Arrange
    let cs = vec![col("id", false), col("c", false)];
    let r = vec![text("1"), text("x")];

    // Act & Assert
    let t = build_update(&TestDialect, &obj(ObjectKind::Table), &cs, &r, &[(1, None)]);
    assert_eq!(t.unwrap_err(), QueryError::NoPrimaryKey);
    let v = build_update(
        &TestDialect,
        &obj(ObjectKind::View),
        &cols(),
        &row(),
        &[(1, None)],
    );
    assert_eq!(v.unwrap_err(), QueryError::NoPrimaryKey);
}

#[test]
fn rejects_bytes_or_truncated_target_and_key() {
    // Arrange
    let bytes = Cell::Bytes {
        value: Box::new([1]),
        total_len: 1,
        truncated: false,
    };
    let trunc = Cell::Text {
        value: "x".into(),
        truncated: true,
    };
    let mut r = row();
    r[1] = bytes.clone();

    // Act & Assert
    assert_eq!(
        build(&r, 1, Some("v")).unwrap_err(),
        QueryError::NotEditable("c".into())
    );
    r[1] = trunc.clone();
    assert_eq!(
        build(&r, 1, Some("v")).unwrap_err(),
        QueryError::NotEditable("c".into())
    );
    let mut r = row();
    r[2] = trunc;
    assert_eq!(
        build(&r, 1, Some("v")).unwrap_err(),
        QueryError::NotEditable("k".into())
    );
    r[2] = bytes;
    assert_eq!(
        build(&r, 1, Some("v")).unwrap_err(),
        QueryError::NotEditable("k".into())
    );
}

/// The server caps text at `CELL_MEMORY_CAP` without flagging it; a full-cap value may be cut.
#[test]
fn rejects_cells_at_memory_cap() {
    // Arrange
    let full = text(&"a".repeat(CELL_MEMORY_CAP));
    let mut r = row();
    r[1] = full.clone();

    // Act & Assert
    assert_eq!(
        build(&r, 1, Some("v")).unwrap_err(),
        QueryError::NotEditable("c".into())
    );
    let mut r = row();
    r[0] = full;
    assert_eq!(
        build(&r, 1, Some("v")).unwrap_err(),
        QueryError::NotEditable("id".into())
    );
}

#[test]
fn rejects_null_pk_cell() {
    // Arrange
    let mut r = row();
    r[0] = Cell::Null;

    // Act & Assert
    assert_eq!(
        build(&r, 1, Some("v")).unwrap_err(),
        QueryError::NotEditable("id".into())
    );
}

#[test]
fn rejects_column_out_of_range() {
    // Act & Assert
    assert!(matches!(
        build(&row(), 3, Some("v")).unwrap_err(),
        QueryError::UnknownColumn(_)
    ));
    assert!(matches!(
        build(&row()[..1], 1, Some("v")).unwrap_err(),
        QueryError::UnknownColumn(_)
    ));
}

fn build_many(row: &[Cell], changes: &[(usize, Option<String>)]) -> Result<BuiltQuery, QueryError> {
    build_update(
        &TestDialect,
        &obj(ObjectKind::Table),
        &cols4(),
        row,
        changes,
    )
}

fn cols4() -> Vec<ColumnMeta> {
    vec![
        col("id", true),
        col("c", false),
        col("x", false),
        col("k", true),
    ]
}

fn row4() -> Vec<Cell> {
    vec![text("1"), text("old"), text("y"), text("b")]
}

#[test]
fn multi_column_update_sets_all_changes_keyed_by_pk() {
    // Arrange
    let changes = [(1, Some("a".to_string())), (2, None)];

    // Act
    let b = build_many(&row4(), &changes).unwrap();

    // Assert
    assert_eq!(
        b.sql,
        r#"UPDATE "public"."t" SET "c" = $1, "x" = $2 WHERE "id" = $3 AND "k" = $4"#
    );
    assert_eq!(
        b.params,
        vec![
            Param(Some("a".into())),
            Param(None),
            Param(Some("1".into())),
            Param(Some("b".into()))
        ]
    );
}

#[test]
fn rejects_empty_changes() {
    // Act & Assert
    assert!(matches!(
        build_many(&row4(), &[]).unwrap_err(),
        QueryError::UnknownColumn(_)
    ));
}

#[test]
fn rejects_opaque_cell_in_any_target() {
    // Arrange
    let mut r = row4();
    r[2] = Cell::Text {
        value: "x".into(),
        truncated: true,
    };

    // Act
    let e = build_many(&r, &[(1, Some("a".into())), (2, Some("z".into()))]);

    // Assert
    assert_eq!(e.unwrap_err(), QueryError::NotEditable("x".into()));
}
