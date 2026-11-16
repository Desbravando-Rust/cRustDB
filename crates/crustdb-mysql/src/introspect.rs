//! Catalog introspection over `information_schema`. Every name is a bound `?` parameter;
//! only `SHOW CREATE` interpolates, through the dialect's identifier quoting.
use async_trait::async_trait;
use crustdb_core::dialect::Dialect;
use crustdb_core::driver::{ExecMode, Introspector, Param, Session};
use crustdb_core::error::{DriverError, ErrorKind};
use crustdb_core::model::{
    ColumnDetail, ColumnMeta, ForeignKeyInfo, IndexInfo, ObjectKind, ObjectRef, TypeCategory,
};

use crate::MysqlDialect;

use crustdb_core::driver::{TextRows, drain_text, text_at};

/// Returns the result's column names and its rows as optional text.
async fn query(
    s: &mut dyn Session,
    sql: &str,
    params: &[&str],
) -> Result<(Vec<String>, TextRows), DriverError> {
    let header = if params.is_empty() {
        s.start(sql, ExecMode::Other).await?
    } else {
        let params: Vec<_> = params.iter().map(|p| Param(Some((*p).into()))).collect();
        s.start_params(sql, &params).await?
    };
    let names = header.columns.into_iter().map(|c| c.name).collect();
    let rows = drain_text(s).await?;
    Ok((names, rows))
}

async fn query_rows(
    s: &mut dyn Session,
    sql: &str,
    params: &[&str],
) -> Result<TextRows, DriverError> {
    Ok(query(s, sql, params).await?.1)
}

fn category(data_type: &str) -> TypeCategory {
    use TypeCategory::*;
    match data_type {
        "tinyint" | "smallint" | "mediumint" | "int" | "integer" | "bigint" | "year" => Integer,
        "decimal" | "numeric" => Decimal,
        "float" | "double" | "real" => Float,
        "date" => Date,
        "time" => Time,
        "datetime" | "timestamp" => Timestamp,
        "json" => Json,
        "binary" | "varbinary" | "tinyblob" | "blob" | "mediumblob" | "longblob" => Binary,
        "bit" | "geometry" | "point" | "linestring" | "polygon" => Other,
        _ => Text,
    }
}

fn objects_sql(kind: ObjectKind) -> Option<&'static str> {
    Some(match kind {
        ObjectKind::Table => {
            "SELECT TABLE_NAME FROM information_schema.TABLES \
             WHERE TABLE_SCHEMA = ? AND TABLE_TYPE = 'BASE TABLE' ORDER BY TABLE_NAME"
        }
        ObjectKind::View => {
            "SELECT TABLE_NAME FROM information_schema.TABLES \
             WHERE TABLE_SCHEMA = ? AND TABLE_TYPE = 'VIEW' ORDER BY TABLE_NAME"
        }
        ObjectKind::Function => {
            "SELECT ROUTINE_NAME FROM information_schema.ROUTINES \
             WHERE ROUTINE_SCHEMA = ? AND ROUTINE_TYPE = 'FUNCTION' ORDER BY ROUTINE_NAME"
        }
        ObjectKind::Procedure => {
            "SELECT ROUTINE_NAME FROM information_schema.ROUTINES \
             WHERE ROUTINE_SCHEMA = ? AND ROUTINE_TYPE = 'PROCEDURE' ORDER BY ROUTINE_NAME"
        }
        ObjectKind::Trigger => {
            "SELECT TRIGGER_NAME FROM information_schema.TRIGGERS \
             WHERE TRIGGER_SCHEMA = ? ORDER BY TRIGGER_NAME"
        }
        _ => return None,
    })
}

fn unsupported_kind(k: ObjectKind) -> DriverError {
    DriverError::new(
        ErrorKind::Unsupported,
        format!("no catalog query for {k:?}"),
    )
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MysqlIntrospector;

#[async_trait]
impl Introspector for MysqlIntrospector {
    async fn databases(&self, s: &mut dyn Session) -> Result<Vec<String>, DriverError> {
        let r = query_rows(
            s,
            "SELECT SCHEMA_NAME FROM information_schema.SCHEMATA ORDER BY SCHEMA_NAME",
            &[],
        )
        .await?;
        Ok(r.iter().map(|r| text_at(r, 0)).collect())
    }

    // MySQL has no schema level.
    async fn schemas(&self, _: &mut dyn Session, _: &str) -> Result<Vec<String>, DriverError> {
        Ok(vec![])
    }

    async fn objects(
        &self,
        s: &mut dyn Session,
        database: &str,
        _: Option<&str>,
        kind: ObjectKind,
    ) -> Result<Vec<ObjectRef>, DriverError> {
        let sql = objects_sql(kind).ok_or_else(|| unsupported_kind(kind))?;
        let r = query_rows(s, sql, &[database]).await?;
        Ok(r.iter()
            .map(|r| ObjectRef {
                database: database.into(),
                schema: None,
                name: text_at(r, 0),
                kind,
            })
            .collect())
    }

    async fn columns(
        &self,
        s: &mut dyn Session,
        o: &ObjectRef,
    ) -> Result<Vec<ColumnDetail>, DriverError> {
        let r = query_rows(
            s,
            "SELECT COLUMN_NAME, COLUMN_TYPE, DATA_TYPE, IS_NULLABLE, COLUMN_DEFAULT, \
                    COLUMN_COMMENT, COLUMN_KEY \
             FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? \
             ORDER BY ORDINAL_POSITION",
            &[&o.database, &o.name],
        )
        .await?;
        Ok(r.iter()
            .map(|r| ColumnDetail {
                meta: ColumnMeta {
                    name: text_at(r, 0),
                    // Display only: MySQL params are `?`, so this is never put into SQL.
                    type_name: text_at(r, 1),
                    category: category(&text_at(r, 2).to_lowercase()),
                    nullable: Some(text_at(r, 3) == "YES"),
                    is_primary_key: text_at(r, 6) == "PRI",
                },
                default: r[4].clone(),
                comment: r[5].clone().filter(|c| !c.is_empty()),
            })
            .collect())
    }

    async fn indexes(
        &self,
        s: &mut dyn Session,
        o: &ObjectRef,
    ) -> Result<Vec<IndexInfo>, DriverError> {
        let r = query_rows(
            s,
            "SELECT INDEX_NAME, NON_UNIQUE, INDEX_TYPE, COLUMN_NAME \
             FROM information_schema.STATISTICS WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? \
             ORDER BY INDEX_NAME, SEQ_IN_INDEX",
            &[&o.database, &o.name],
        )
        .await?;
        let mut out: Vec<IndexInfo> = Vec::new();
        for r in &r {
            let name = text_at(r, 0);
            if out.last().is_none_or(|l| l.name != name) {
                out.push(IndexInfo {
                    primary: name == "PRIMARY",
                    unique: text_at(r, 1) == "0",
                    method: r[2].clone(),
                    name,
                    columns: vec![],
                });
            }
            if let Some(l) = out.last_mut() {
                l.columns.push(text_at(r, 3));
            }
        }
        Ok(out)
    }

    async fn foreign_keys(
        &self,
        s: &mut dyn Session,
        o: &ObjectRef,
    ) -> Result<Vec<ForeignKeyInfo>, DriverError> {
        let r = query_rows(
            s,
            "SELECT k.CONSTRAINT_NAME, rc.UPDATE_RULE, rc.DELETE_RULE, k.REFERENCED_TABLE_SCHEMA, \
                    k.REFERENCED_TABLE_NAME, k.COLUMN_NAME, k.REFERENCED_COLUMN_NAME \
             FROM information_schema.KEY_COLUMN_USAGE k \
             JOIN information_schema.REFERENTIAL_CONSTRAINTS rc \
               ON rc.CONSTRAINT_SCHEMA = k.CONSTRAINT_SCHEMA \
              AND rc.CONSTRAINT_NAME = k.CONSTRAINT_NAME AND rc.TABLE_NAME = k.TABLE_NAME \
             WHERE k.TABLE_SCHEMA = ? AND k.TABLE_NAME = ? AND k.REFERENCED_TABLE_NAME IS NOT NULL \
             ORDER BY k.CONSTRAINT_NAME, k.ORDINAL_POSITION",
            &[&o.database, &o.name],
        )
        .await?;
        let mut out: Vec<ForeignKeyInfo> = Vec::new();
        for r in &r {
            let name = text_at(r, 0);
            if out.last().is_none_or(|l| l.name != name) {
                out.push(ForeignKeyInfo {
                    name,
                    columns: vec![],
                    ref_table: ObjectRef {
                        database: text_at(r, 3),
                        schema: None,
                        name: text_at(r, 4),
                        kind: ObjectKind::Table,
                    },
                    ref_columns: vec![],
                    on_update: text_at(r, 1),
                    on_delete: text_at(r, 2),
                });
            }
            if let Some(l) = out.last_mut() {
                l.columns.push(text_at(r, 5));
                l.ref_columns.push(text_at(r, 6));
            }
        }
        Ok(out)
    }

    async fn ddl(&self, s: &mut dyn Session, o: &ObjectRef) -> Result<String, DriverError> {
        let (what, column) = match o.kind {
            ObjectKind::Table => ("TABLE", "Create Table"),
            ObjectKind::View => ("VIEW", "Create View"),
            ObjectKind::Function => ("FUNCTION", "Create Function"),
            ObjectKind::Procedure => ("PROCEDURE", "Create Procedure"),
            ObjectKind::Trigger => ("TRIGGER", "SQL Original Statement"),
            k => return Err(unsupported_kind(k)),
        };
        let sql = format!("SHOW CREATE {what} {}", MysqlDialect.qualified_name(o));
        let (names, rows) = query(s, &sql, &[]).await?;
        let i = names
            .iter()
            .position(|n| n.eq_ignore_ascii_case(column))
            .ok_or_else(|| {
                DriverError::new(
                    ErrorKind::Protocol,
                    format!("SHOW CREATE has no `{column}` column"),
                )
            })?;
        Ok(rows.first().map(|r| text_at(r, i)).unwrap_or_default())
    }

    async fn row_estimate(
        &self,
        s: &mut dyn Session,
        o: &ObjectRef,
    ) -> Result<Option<u64>, DriverError> {
        let r = query_rows(
            s,
            "SELECT TABLE_ROWS FROM information_schema.TABLES \
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?",
            &[&o.database, &o.name],
        )
        .await?;
        Ok(r.first().and_then(|r| r[0].as_deref()?.parse().ok()))
    }
}
