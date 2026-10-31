//! Keyset pagination: predicate for `TableQuery::after` and the cursor of the next page (ADR 0011).

use crate::dialect::Dialect;
use crate::driver::Param;
use crate::edit::is_opaque;
use crate::model::{Cell, TypeCategory};
use crate::query::{QueryError, TableQuery};

/// `WHERE` fragment selecting the rows after `q.after` in `q.sorts` order. Values are pushed
/// to `params` (numbered after the ones already there). Same direction for every sort column
/// gives a row-value comparison; mixed directions expand into `OR`s.
pub(crate) fn predicate(
    d: &dyn Dialect,
    q: &TableQuery,
    base: &str,
    params: &mut Vec<Param>,
) -> Result<String, QueryError> {
    let after = q.after.as_deref().unwrap_or_default();
    if q.sorts.is_empty() || after.len() != q.sorts.len() {
        return Err(QueryError::CursorLen {
            want: q.sorts.len(),
            got: after.len(),
        });
    }
    let mut keys = Vec::new();
    for s in &q.sorts {
        let c = q
            .columns
            .iter()
            .find(|c| c.name == s.column)
            .ok_or_else(|| QueryError::UnknownColumn(s.column.clone()))?;
        keys.push((
            format!("{base}.{}", d.quote_ident(&c.name)),
            c,
            s.descending,
        ));
    }
    let mut bind = |i: usize| {
        params.push(Param(Some(after[i].clone())));
        d.param(params.len(), keys[i].1)
    };
    let op = |desc: bool| if desc { "<" } else { ">" };
    let first = keys[0].2;
    if keys.iter().all(|k| k.2 == first) {
        let cols: Vec<_> = keys.iter().map(|k| k.0.clone()).collect();
        let vals: Vec<_> = (0..keys.len()).map(&mut bind).collect();
        return Ok(format!(
            "({}) {} ({})",
            cols.join(", "),
            op(first),
            vals.join(", ")
        ));
    }
    let mut ors = Vec::new();
    for i in 0..keys.len() {
        let mut ands: Vec<String> = (0..i)
            .map(|j| format!("{} = {}", keys[j].0, bind(j)))
            .collect();
        ands.push(format!("{} {} {}", keys[i].0, op(keys[i].2), bind(i)));
        ors.push(format!("({})", ands.join(" AND ")));
    }
    Ok(format!("({})", ors.join(" OR ")))
}

/// Cursor for the page after the one ending in `row` (cells in `q.columns` order), or `None`
/// when the order is not safe for keyset and the caller must keep using `OFFSET`.
pub fn after(q: &TableQuery, row: &[Cell]) -> Option<Vec<String>> {
    if q.sorts.is_empty() {
        return None;
    }
    let mut cursor = Vec::new();
    for s in &q.sorts {
        let i = q.columns.iter().position(|c| c.name == s.column)?;
        let c = &q.columns[i];
        let ordered = matches!(
            c.category,
            TypeCategory::Integer
                | TypeCategory::Decimal
                | TypeCategory::Text
                | TypeCategory::Bool
                | TypeCategory::Date
                | TypeCategory::Time
                | TypeCategory::Timestamp
                | TypeCategory::Uuid
        );
        if !ordered || !(c.is_primary_key || c.nullable == Some(false)) {
            return None;
        }
        match row.get(i)? {
            cell @ Cell::Text { value, .. } if !is_opaque(cell) => cursor.push(value.to_string()),
            _ => return None,
        }
    }
    let pk = q.columns.iter().filter(|c| c.is_primary_key);
    let mut any = false;
    for c in pk {
        any = true;
        if !q.sorts.iter().any(|s| s.column == c.name) {
            return None;
        }
    }
    any.then_some(cursor)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::model::{ColumnMeta, ObjectKind, ObjectRef};
    use crate::query::Sort;

    fn col(name: &str, cat: TypeCategory, pk: bool, nullable: Option<bool>) -> ColumnMeta {
        ColumnMeta {
            name: name.into(),
            type_name: "t".into(),
            category: cat,
            nullable,
            is_primary_key: pk,
        }
    }

    fn sort(c: &str, descending: bool) -> Sort {
        Sort {
            column: c.into(),
            descending,
        }
    }

    fn q(sorts: Vec<Sort>) -> TableQuery {
        TableQuery {
            object: ObjectRef {
                database: "db".into(),
                schema: None,
                name: "t".into(),
                kind: ObjectKind::Table,
            },
            columns: vec![
                col("id", TypeCategory::Integer, true, Some(false)),
                col("a", TypeCategory::Text, false, Some(false)),
                col("n", TypeCategory::Text, false, None),
                col("f", TypeCategory::Float, false, Some(false)),
            ],
            filters: vec![],
            sorts,
            limit: Some(8),
            offset: 0,
            after: None,
        }
    }

    fn t(s: &str) -> Cell {
        Cell::Text {
            value: s.into(),
            truncated: false,
        }
    }

    fn row() -> Vec<Cell> {
        vec![t("5"), t("x"), t("y"), t("1.5")]
    }

    #[test]
    fn cursor_follows_sort_order() {
        // Act & Assert
        let got = after(&q(vec![sort("a", true), sort("id", false)]), &row());
        assert_eq!(got, Some(vec!["x".to_string(), "5".to_string()]));
    }

    #[test]
    fn cursor_none_when_keyset_unsafe() {
        // Arrange
        let mut opaque = row();
        opaque[0] = Cell::Text {
            value: "5".into(),
            truncated: true,
        };
        let mut null = row();
        null[1] = Cell::Null;

        // Act & Assert
        let pk = vec![sort("id", false)];
        assert!(after(&q(vec![]), &row()).is_none(), "no sorts");
        assert!(
            after(&q(vec![sort("a", false)]), &row()).is_none(),
            "pk missing"
        );
        assert!(
            after(&q(vec![sort("n", false), sort("id", false)]), &row()).is_none(),
            "nullable sort column"
        );
        assert!(
            after(&q(vec![sort("f", false), sort("id", false)]), &row()).is_none(),
            "float"
        );
        assert!(after(&q(pk.clone()), &opaque).is_none(), "opaque cell");
        assert!(
            after(&q(vec![sort("a", false), sort("id", false)]), &null).is_none(),
            "null cell"
        );
        let mut nopk = q(pk);
        nopk.columns[0].is_primary_key = false;
        assert!(after(&nopk, &row()).is_none(), "no primary key");
    }
}
