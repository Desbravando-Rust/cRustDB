//! Single-cell `UPDATE` keyed by every primary-key column (ADR 0003).

use crate::dialect::{BuiltQuery, Dialect};
use crate::driver::Param;
use crate::model::{CELL_MEMORY_CAP, Cell, ColumnMeta, ObjectKind, ObjectRef};
use crate::query::QueryError;

/// `row` holds the page cells in the order of `columns`. `value = None` writes `NULL`.
pub fn build_update(
    d: &dyn Dialect,
    object: &ObjectRef,
    columns: &[ColumnMeta],
    row: &[Cell],
    column: usize,
    value: Option<String>,
) -> Result<BuiltQuery, QueryError> {
    let unknown = || QueryError::UnknownColumn(format!("#{column}"));
    let target = columns.get(column).ok_or_else(unknown)?;
    let target_cell = row.get(column).ok_or_else(unknown)?;
    if object.kind != ObjectKind::Table || !columns.iter().any(|c| c.is_primary_key) {
        return Err(QueryError::NoPrimaryKey);
    }
    if is_opaque(target_cell) {
        return Err(QueryError::NotEditable(target.name.clone()));
    }

    let mut params = vec![Param(value)];
    let mut sql = format!(
        "UPDATE {} SET {} = {} WHERE ",
        d.qualified_name(object),
        d.quote_ident(&target.name),
        d.param(1, target)
    );
    let mut conds = Vec::new();
    for (i, c) in columns.iter().enumerate().filter(|(_, c)| c.is_primary_key) {
        let key = match row.get(i) {
            Some(cell @ Cell::Text { value, .. }) if !is_opaque(cell) => value.to_string(),
            _ => return Err(QueryError::NotEditable(c.name.clone())),
        };
        params.push(Param(Some(key)));
        conds.push(format!(
            "{} = {}",
            d.quote_ident(&c.name),
            d.param(params.len(), c)
        ));
    }
    sql.push_str(&conds.join(" AND "));
    Ok(BuiltQuery { sql, params })
}

/// Binary, truncated, or text at `CELL_MEMORY_CAP` (servers cap it without flagging): the
/// value held may not be the stored one, so it can neither be edited nor key a row.
pub fn is_opaque(c: &Cell) -> bool {
    match c {
        Cell::Null => false,
        Cell::Text { value, truncated } => *truncated || value.len() >= CELL_MEMORY_CAP,
        Cell::Bytes { .. } => true,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
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
            column,
            v.map(String::from),
        )
    }

    #[test]
    fn update_binds_value_and_all_pk_columns() {
        let b = build(&row(), 1, Some("new")).unwrap();
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
        let b = build(&row(), 1, None).unwrap();
        assert_eq!(b.params[0], Param(None));
    }

    #[test]
    fn rejects_table_without_pk() {
        let cs = vec![col("id", false), col("c", false)];
        let r = vec![text("1"), text("x")];
        let t = build_update(&TestDialect, &obj(ObjectKind::Table), &cs, &r, 1, None);
        assert_eq!(t.unwrap_err(), QueryError::NoPrimaryKey);
        let v = build_update(
            &TestDialect,
            &obj(ObjectKind::View),
            &cols(),
            &row(),
            1,
            None,
        );
        assert_eq!(v.unwrap_err(), QueryError::NoPrimaryKey);
    }

    #[test]
    fn rejects_bytes_or_truncated_target_and_key() {
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
        let full = text(&"a".repeat(CELL_MEMORY_CAP));
        let mut r = row();
        r[1] = full.clone();
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
        let mut r = row();
        r[0] = Cell::Null;
        assert_eq!(
            build(&r, 1, Some("v")).unwrap_err(),
            QueryError::NotEditable("id".into())
        );
    }

    #[test]
    fn rejects_column_out_of_range() {
        assert!(matches!(
            build(&row(), 3, Some("v")).unwrap_err(),
            QueryError::UnknownColumn(_)
        ));
        assert!(matches!(
            build(&row()[..1], 1, Some("v")).unwrap_err(),
            QueryError::UnknownColumn(_)
        ));
    }
}
