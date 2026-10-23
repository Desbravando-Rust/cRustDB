//! Catalog introspection over `sqlite_schema` and `pragma_table_info`. Every name is a bound
//! parameter. Only the `main` database is listed (no ATTACH).
use async_trait::async_trait;
use crustdb_core::driver::{Introspector, Param, Session};
use crustdb_core::error::{DriverError, ErrorKind};
use crustdb_core::model::{
    Cell, ColumnDetail, ColumnMeta, ForeignKeyInfo, IndexInfo, ObjectKind, ObjectRef,
};

use crate::connect::err;
use crate::types::category;

type Rows = Vec<Vec<Option<String>>>;

/// Runs a parameterized query and returns its rows as optional text.
async fn query_rows(s: &mut dyn Session, sql: &str, params: &[&str]) -> Result<Rows, DriverError> {
    let params: Vec<_> = params.iter().map(|p| Param(Some((*p).into()))).collect();
    s.start_params(sql, &params).await?;
    let mut rows = Rows::new();
    loop {
        let f = match s.fetch(1000).await {
            Ok(f) => f,
            Err(e) => {
                let _ = s.finish().await;
                return Err(e);
            }
        };
        rows.extend(f.rows.iter().map(|r| {
            r.iter()
                .map(|c| match c {
                    Cell::Text { value, .. } => Some(value.to_string()),
                    _ => None,
                })
                .collect()
        }));
        if f.exhausted {
            break;
        }
    }
    s.finish().await?;
    Ok(rows)
}

fn col(r: &[Option<String>], i: usize) -> String {
    r.get(i).cloned().flatten().unwrap_or_default()
}

/// `sqlite_schema.type` of the kinds shown in the tree.
fn schema_type(kind: ObjectKind) -> Result<&'static str, DriverError> {
    match kind {
        ObjectKind::Table => Ok("table"),
        ObjectKind::View => Ok("view"),
        ObjectKind::Trigger => Ok("trigger"),
        k => Err(err(
            ErrorKind::Unsupported,
            format!("no catalog query for {k:?}"),
        )),
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SqliteIntrospector;

#[async_trait]
impl Introspector for SqliteIntrospector {
    async fn databases(&self, _: &mut dyn Session) -> Result<Vec<String>, DriverError> {
        Ok(vec!["main".into()])
    }

    // SQLite has no schema level.
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
        let r = query_rows(
            s,
            "SELECT name FROM sqlite_schema \
             WHERE type = ?1 AND name NOT LIKE 'sqlite!_%' ESCAPE '!' ORDER BY name",
            &[schema_type(kind)?],
        )
        .await?;
        Ok(r.iter()
            .map(|r| ObjectRef {
                database: database.into(),
                schema: None,
                name: col(r, 0),
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
            "SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info(?1) ORDER BY cid",
            &[&o.name],
        )
        .await?;
        Ok(r.iter()
            .map(|r| {
                let type_name = col(r, 1);
                ColumnDetail {
                    meta: ColumnMeta {
                        name: col(r, 0),
                        category: category(Some(type_name.as_str())),
                        type_name,
                        nullable: Some(col(r, 2) == "0"),
                        is_primary_key: col(r, 4) != "0",
                    },
                    default: r.get(3).cloned().flatten(),
                    comment: None,
                }
            })
            .collect())
    }

    // ponytail: no indexes/FKs in the tree yet (out of scope for #26).
    async fn indexes(
        &self,
        _: &mut dyn Session,
        _: &ObjectRef,
    ) -> Result<Vec<IndexInfo>, DriverError> {
        Ok(vec![])
    }

    async fn foreign_keys(
        &self,
        _: &mut dyn Session,
        _: &ObjectRef,
    ) -> Result<Vec<ForeignKeyInfo>, DriverError> {
        Ok(vec![])
    }

    async fn ddl(&self, s: &mut dyn Session, o: &ObjectRef) -> Result<String, DriverError> {
        let r = query_rows(
            s,
            "SELECT sql FROM sqlite_schema WHERE type = ?1 AND name = ?2",
            &[schema_type(o.kind)?, &o.name],
        )
        .await?;
        Ok(r.first().map(|r| col(r, 0)).unwrap_or_default())
    }

    // ponytail: unknown until `sqlite_stat1` is read.
    async fn row_estimate(
        &self,
        _: &mut dyn Session,
        _: &ObjectRef,
    ) -> Result<Option<u64>, DriverError> {
        Ok(None)
    }
}
