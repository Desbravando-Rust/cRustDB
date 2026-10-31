//! Row `UPDATE`/`INSERT`/`DELETE` keyed by every primary-key column (ADR 0003, ADR 0004).

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

/// `INSERT` with only the given `(column, value)` pairs; omitted columns take the database
/// default and `None` writes `NULL`. Same gate as `build_update`: tables with a PK only.
///
/// Errors: `UnknownColumn` for empty `values` or an out-of-range column, `NoPrimaryKey`.
pub fn build_insert(
    d: &dyn Dialect,
    object: &ObjectRef,
    columns: &[ColumnMeta],
    values: &[(usize, Option<String>)],
) -> Result<BuiltQuery, QueryError> {
    if values.is_empty() {
        return Err(QueryError::UnknownColumn("(no values)".into()));
    }
    require_pk(object, columns)?;
    let mut params = Vec::new();
    let mut names = Vec::new();
    let mut holders = Vec::new();
    for (column, value) in values {
        let target = columns
            .get(*column)
            .ok_or_else(|| QueryError::UnknownColumn(format!("#{column}")))?;
        params.push(Param(value.clone()));
        names.push(d.quote_ident(&target.name));
        holders.push(d.param(params.len(), target));
    }
    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({})",
        d.qualified_name(object),
        names.join(", "),
        holders.join(", ")
    );
    Ok(BuiltQuery { sql, params })
}

/// One `DELETE` for all `rows`: `WHERE (pk AND ..) OR (..)`, each row keyed by every PK column.
///
/// Errors: `UnknownColumn` for empty `rows`, `NoPrimaryKey`, `NotEditable` for a null or
/// opaque key cell.
// ponytail: one statement for the whole selection; bounded by the page size, under the lowest
// parameter cap (SQLite, 32766). Chunk the rows if pages ever outgrow it.
pub fn build_delete(
    d: &dyn Dialect,
    object: &ObjectRef,
    columns: &[ColumnMeta],
    rows: &[&[Cell]],
) -> Result<BuiltQuery, QueryError> {
    if rows.is_empty() {
        return Err(QueryError::UnknownColumn("(no rows)".into()));
    }
    require_pk(object, columns)?;
    let mut params = Vec::new();
    let mut groups = Vec::new();
    for row in rows {
        groups.push(format!("({})", key_where(d, columns, row, &mut params)?));
    }
    let sql = format!(
        "DELETE FROM {} WHERE {}",
        d.qualified_name(object),
        groups.join(" OR ")
    );
    Ok(BuiltQuery { sql, params })
}

fn require_pk(object: &ObjectRef, columns: &[ColumnMeta]) -> Result<(), QueryError> {
    if object.kind != ObjectKind::Table || !columns.iter().any(|c| c.is_primary_key) {
        return Err(QueryError::NoPrimaryKey);
    }
    Ok(())
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
