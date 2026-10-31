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
        // A PK counts as not null unless introspection says otherwise (SQLite rowid tables
        // accept NULL in a non-INTEGER PK).
        let not_null = c.nullable == Some(false) || (c.is_primary_key && c.nullable != Some(true));
        // MySQL ENUM/SET are Text but sort by declaration index, not by the string.
        let ty = c.type_name.to_ascii_lowercase();
        if !ordered || !not_null || ty.starts_with("enum(") || ty.starts_with("set(") {
            return None;
        }
        match row.get(i)? {
            cell @ Cell::Text { value, .. } if !is_opaque(cell) => {
                if !exact_as_double(c.category, value) {
                    return None;
                }
                cursor.push(value.to_string());
            }
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

/// ponytail: MySQL binds the cursor as text, so an integer/decimal key compares as a double
/// and skips or repeats rows beyond 2^53 / ~15 digits. Applied to every dialect (it only
/// costs huge keys); upgrade path is a typed bind in `MysqlDialect::param`.
fn exact_as_double(cat: TypeCategory, v: &str) -> bool {
    match cat {
        TypeCategory::Integer => v.parse::<i128>().is_ok_and(|n| n.unsigned_abs() <= 1 << 53),
        TypeCategory::Decimal => {
            v.chars()
                .filter(char::is_ascii_digit)
                .skip_while(|&d| d == '0')
                .count()
                <= 15
        }
        _ => true,
    }
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

    fn cursor_for(id: &str, mutate: impl FnOnce(&mut TableQuery)) -> Option<Vec<String>> {
        let mut query = q(vec![sort("id", false)]);
        mutate(&mut query);
        after(&query, &[t(id), t("x"), t("y"), t("1.5")])
    }

    #[test]
    fn cursor_none_for_enum_and_set() {
        // Act & Assert
        for ty in ["enum('a','b')", "SET('a','b')"] {
            let got = cursor_for("5", |q| {
                q.columns[1].type_name = ty.into();
                q.sorts = vec![sort("a", false), sort("id", false)];
            });
            assert!(got.is_none(), "{ty}");
        }
    }

    #[test]
    fn cursor_none_for_keys_beyond_double_precision() {
        // Act & Assert
        assert!(cursor_for("9007199254740992", |_| {}).is_some());
        assert!(cursor_for("-9007199254740992", |_| {}).is_some());
        assert!(cursor_for("9007199254740993", |_| {}).is_none());
        let dec = |v: &str| {
            cursor_for(v, |q| {
                q.columns[0].category = TypeCategory::Decimal;
            })
        };
        assert!(dec("-123456789012.345").is_some());
        assert!(dec("1234567890123.4567").is_none());
    }

    #[test]
    fn primary_key_nullability_follows_introspection() {
        // Act & Assert
        assert!(cursor_for("5", |q| q.columns[0].nullable = None).is_some());
        assert!(cursor_for("5", |q| q.columns[0].nullable = Some(true)).is_none());
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
