//! Statement execution for `PgSession`: server cursor, simple-protocol and typed streams.
use std::pin::Pin;

use crustdb_core::driver::{Fetched, Param};
use crustdb_core::error::{DriverError, ErrorKind};
use crustdb_core::model::{CELL_MEMORY_CAP, Cell, ColumnMeta, Completion, Row, TypeCategory};
use futures_util::StreamExt;
use tokio_postgres::types::{ToSql, Type};
use tokio_postgres::{RowStream, SimpleQueryMessage, SimpleQueryStream, Statement};

use crate::connect::{err, map_err};
use crate::session::PgSession;
use crate::types::category_for_oid;

pub(crate) enum Active {
    Cursor { implicit_tx: bool },
    Simple(Pin<Box<SimpleQueryStream>>),
    Typed(Pin<Box<RowStream>>),
}

fn meta(name: &str, type_name: &str, category: TypeCategory) -> ColumnMeta {
    ColumnMeta {
        name: name.into(),
        type_name: type_name.into(),
        category,
        nullable: None,
        is_primary_key: false,
    }
}

fn stmt_meta(s: &Statement) -> Vec<ColumnMeta> {
    s.columns()
        .iter()
        .map(|c| {
            meta(
                c.name(),
                c.type_().name(),
                category_for_oid(c.type_().oid()),
            )
        })
        .collect()
}

fn nibble(b: u8) -> u8 {
    char::from(b).to_digit(16).map_or(0, |d| d as u8)
}

/// Decodes a `\x…` bytea literal, keeping at most `CELL_MEMORY_CAP` bytes.
fn bytea_cell(hex: &str) -> Cell {
    let h = hex.as_bytes().get(2..).unwrap_or_default();
    let total = h.len() / 2;
    let keep = total.min(CELL_MEMORY_CAP);
    let value = h
        .as_chunks::<2>()
        .0
        .iter()
        .take(keep)
        .map(|[a, b]| (nibble(*a) << 4) | nibble(*b))
        .collect();
    Cell::Bytes {
        value,
        total_len: total as u64,
        truncated: keep < total,
    }
}

fn cell(v: Option<&str>, cat: TypeCategory) -> Cell {
    match v {
        None => Cell::Null,
        Some(s) if cat == TypeCategory::Binary && s.starts_with("\\x") => bytea_cell(s),
        Some(s) => Cell::text_capped(s),
    }
}

/// A `DbError` (e.g. aborted transaction on CLOSE) is not a reason to fail cleanup.
fn ignore_db_error(r: Result<(), tokio_postgres::Error>) -> Result<(), DriverError> {
    r.or_else(|e| {
        if e.as_db_error().is_some() {
            Ok(())
        } else {
            Err(map_err(e))
        }
    })
}

fn modify_rows(v: &serde_json::Value) -> Option<u64> {
    use serde_json::Value;
    match v {
        Value::Object(o) if o.get("Node Type").and_then(Value::as_str) == Some("ModifyTable") => {
            o.get("Plans")?.get(0)?.get("Plan Rows")?.as_u64()
        }
        Value::Object(o) => o.values().find_map(modify_rows),
        Value::Array(a) => a.iter().find_map(modify_rows),
        _ => None,
    }
}

impl PgSession {
    pub(crate) async fn start_cursor(
        &mut self,
        sql: &str,
        stmt: Option<&Statement>,
    ) -> Result<Vec<ColumnMeta>, DriverError> {
        let implicit_tx = !self.in_user_tx;
        if implicit_tx {
            self.client.batch_execute("BEGIN").await.map_err(map_err)?;
        }
        let declared = async {
            self.client
                .batch_execute(&format!("DECLARE crustdb_cur NO SCROLL CURSOR FOR {sql}"))
                .await
                .map_err(map_err)?;
            if let Some(s) = stmt {
                return Ok(stmt_meta(s));
            }
            // No Describe: take the names from an empty FETCH.
            let msgs = self
                .client
                .simple_query("FETCH 0 FROM crustdb_cur")
                .await
                .map_err(map_err)?;
            Ok(msgs
                .iter()
                .find_map(|m| match m {
                    SimpleQueryMessage::RowDescription(c) => Some(
                        c.iter()
                            .map(|c| meta(c.name(), "", TypeCategory::Other))
                            .collect(),
                    ),
                    _ => None,
                })
                .unwrap_or_default())
        }
        .await;
        match declared {
            Ok(cols) => {
                self.cats = cols.iter().map(|c| c.category).collect();
                self.active = Some(Active::Cursor { implicit_tx });
                Ok(cols)
            }
            Err(e) => {
                if implicit_tx {
                    let _ = self.client.batch_execute("ROLLBACK").await;
                }
                Err(e)
            }
        }
    }

    pub(crate) async fn start_simple(
        &mut self,
        sql: &str,
        stmt: Option<&Statement>,
    ) -> Result<Vec<ColumnMeta>, DriverError> {
        self.affected = None;
        let mut st = Box::pin(self.client.simple_query_raw(sql).await.map_err(map_err)?);
        let mut cols = Vec::new();
        match st.next().await.transpose().map_err(map_err)? {
            Some(SimpleQueryMessage::RowDescription(c)) => {
                cols = match stmt {
                    Some(s) if s.columns().len() == c.len() => stmt_meta(s),
                    _ => c
                        .iter()
                        .map(|c| meta(c.name(), "", TypeCategory::Other))
                        .collect(),
                }
            }
            Some(SimpleQueryMessage::CommandComplete(n)) => self.affected = Some(n),
            _ => {}
        }
        self.cats = cols.iter().map(|c| c.category).collect();
        self.active = Some(Active::Simple(st));
        Ok(cols)
    }

    /// Grid path: every column is already `::text`, parameters are bound as text.
    pub(crate) async fn start_typed(
        &mut self,
        sql: &str,
        params: &[Param],
    ) -> Result<Vec<ColumnMeta>, DriverError> {
        let types = vec![Type::TEXT; params.len()];
        let stmt = self
            .client
            .prepare_typed(sql, &types)
            .await
            .map_err(map_err)?;
        let st = self
            .client
            .query_raw(&stmt, params.iter().map(|p| &p.0 as &(dyn ToSql + Sync)))
            .await
            .map_err(map_err)?;
        let cols: Vec<_> = stmt
            .columns()
            .iter()
            .map(|c| meta(c.name(), "text", TypeCategory::Text))
            .collect();
        self.cats = vec![TypeCategory::Text; cols.len()];
        self.active = Some(Active::Typed(Box::pin(st)));
        Ok(cols)
    }

    pub(crate) async fn fetch_rows(&mut self, n: usize) -> Result<Fetched, DriverError> {
        let Self {
            client,
            active,
            cats,
            affected,
            ..
        } = self;
        let cat = |i: usize| cats.get(i).copied().unwrap_or(TypeCategory::Other);
        let mut rows: Vec<Row> = Vec::new();
        let mut exhausted = false;
        match active {
            None => {
                return Ok(Fetched {
                    rows,
                    exhausted: true,
                });
            }
            Some(Active::Cursor { .. }) => {
                let msgs = client
                    .simple_query(&format!("FETCH {n} FROM crustdb_cur"))
                    .await
                    .map_err(map_err)?;
                for m in &msgs {
                    if let SimpleQueryMessage::Row(r) = m {
                        rows.push((0..r.len()).map(|i| cell(r.get(i), cat(i))).collect());
                    }
                }
                exhausted = rows.len() < n;
            }
            Some(Active::Simple(st)) => {
                while rows.len() < n {
                    match st.next().await.transpose().map_err(map_err)? {
                        None => {
                            exhausted = true;
                            break;
                        }
                        Some(SimpleQueryMessage::Row(r)) => {
                            rows.push((0..r.len()).map(|i| cell(r.get(i), cat(i))).collect());
                        }
                        Some(SimpleQueryMessage::CommandComplete(c)) => *affected = Some(c),
                        Some(_) => {}
                    }
                }
            }
            Some(Active::Typed(st)) => {
                while rows.len() < n {
                    let Some(r) = st.next().await.transpose().map_err(map_err)? else {
                        *affected = st.rows_affected();
                        exhausted = true;
                        break;
                    };
                    let mut cells = Vec::with_capacity(r.len());
                    for i in 0..r.len() {
                        let v = r.try_get::<_, Option<&str>>(i).map_err(map_err)?;
                        cells.push(cell(v, cat(i)));
                    }
                    rows.push(cells.into());
                }
            }
        }
        // A finished stream must not be polled again (it would report "connection closed").
        if exhausted && !matches!(active, Some(Active::Cursor { .. })) {
            *active = None;
        }
        Ok(Fetched { rows, exhausted })
    }

    /// D37: closes the cursor (and its implicit transaction) or drains the stream (D29).
    pub(crate) async fn finish_active(&mut self) -> Result<Completion, DriverError> {
        match self.active.take() {
            None => {}
            Some(Active::Cursor { implicit_tx }) => {
                let close = ignore_db_error(self.client.batch_execute("CLOSE crustdb_cur").await);
                let commit = if implicit_tx {
                    self.client.batch_execute("COMMIT").await.map_err(map_err)
                } else {
                    Ok(())
                };
                close?;
                commit?;
            }
            Some(Active::Simple(mut st)) => {
                while let Some(m) = st.next().await.transpose().map_err(map_err)? {
                    if let SimpleQueryMessage::CommandComplete(c) = m {
                        self.affected = Some(c);
                    }
                }
            }
            Some(Active::Typed(mut st)) => {
                while st.next().await.transpose().map_err(map_err)?.is_some() {}
                self.affected = st.rows_affected();
            }
        }
        Ok(Completion {
            affected_rows: self.affected.take(),
        })
    }

    pub(crate) async fn explain(&mut self, sql: &str) -> Result<Option<u64>, DriverError> {
        let msgs = self
            .client
            .simple_query(&format!("EXPLAIN (FORMAT JSON) {sql}"))
            .await
            .map_err(map_err)?;
        let json = msgs.iter().find_map(|m| match m {
            SimpleQueryMessage::Row(r) => r.get(0),
            _ => None,
        });
        let plan: serde_json::Value = serde_json::from_str(json.unwrap_or("null"))
            .map_err(|e| err(ErrorKind::Protocol, format!("bad EXPLAIN output: {e}")))?;
        Ok(modify_rows(&plan))
    }
}
