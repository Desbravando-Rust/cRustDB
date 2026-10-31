//! Row `UPDATE` keyed by every primary-key column (ADR 0003, ADR 0004).

use crate::dialect::{BuiltQuery, Dialect};
use crate::driver::Param;
use crate::model::{CELL_MEMORY_CAP, Cell, ColumnMeta, ObjectKind, ObjectRef};
use crate::query::QueryError;

/// `row` holds the page cells in the order of `columns`; `changes` are `(column, value)` pairs
/// for that row, and `value = None` writes `NULL`. The `WHERE` uses the original key cells.
///
/// Errors: `UnknownColumn` for an empty `changes` or an out-of-range column, `NoPrimaryKey`
/// for a view or a table without a key, `NotEditable` for an opaque target or key cell.
pub fn build_update(
    d: &dyn Dialect,
    object: &ObjectRef,
    columns: &[ColumnMeta],
    row: &[Cell],
    changes: &[(usize, Option<String>)],
) -> Result<BuiltQuery, QueryError> {
    if changes.is_empty() {
        return Err(QueryError::UnknownColumn("(no changes)".into()));
    }
    let mut params = Vec::new();
    let mut sets = Vec::new();
    for (column, value) in changes {
        let unknown = || QueryError::UnknownColumn(format!("#{column}"));
        let target = columns.get(*column).ok_or_else(unknown)?;
        let target_cell = row.get(*column).ok_or_else(unknown)?;
        if object.kind != ObjectKind::Table || !columns.iter().any(|c| c.is_primary_key) {
            return Err(QueryError::NoPrimaryKey);
        }
        if is_opaque(target_cell) {
            return Err(QueryError::NotEditable(target.name.clone()));
        }
        params.push(Param(value.clone()));
        sets.push(format!(
            "{} = {}",
            d.quote_ident(&target.name),
            d.param(params.len(), target)
        ));
    }

    let conds = key_where(d, columns, row, &mut params)?;
    let sql = format!(
        "UPDATE {} SET {} WHERE {}",
        d.qualified_name(object),
        sets.join(", "),
        conds
    );
    Ok(BuiltQuery { sql, params })
}

/// `col = $n AND ...` over every PK column of `row`, pushing the key values onto `params`.
fn key_where(
    d: &dyn Dialect,
    columns: &[ColumnMeta],
    row: &[Cell],
    params: &mut Vec<Param>,
) -> Result<String, QueryError> {
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
    Ok(conds.join(" AND "))
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
#[path = "edit_tests.rs"]
mod tests;
