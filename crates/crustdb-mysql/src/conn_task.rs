//! The task that owns the `Conn`: a paused `QueryResult` borrows it, so it cannot live in the
//! session handle. Commands arrive over a channel; while a result is active only
//! Fetch/Finish are served and any other command finishes it first (D37).
use std::sync::Arc;
use std::time::Duration;

use crustdb_core::dialect::Dialect;
use crustdb_core::driver::{CancelHandle, ExecMode, Fetched, Param, ResultHeader};
use crustdb_core::error::DriverError;
use crustdb_core::model::{ColumnMeta, Completion, DriverKind, Row};
use crustdb_core::sql::classify;
use mysql_async::consts::ColumnFlags;
use mysql_async::prelude::{Protocol, Queryable};
use mysql_async::{Column, Conn, Opts, Params, QueryResult, Value};
use tokio::sync::{mpsc, oneshot};

use crate::connect::map_err;
use crate::dialect::MysqlDialect;
use crate::session::MysqlCancel;
use crate::types::{ColInfo, nullable, type_name, value_to_cell};

pub(crate) type Reply<T> = oneshot::Sender<Result<T, DriverError>>;

pub(crate) enum ConnCmd {
    Start {
        sql: String,
        mode: ExecMode,
        reply: Reply<ResultHeader>,
    },
    StartParams {
        sql: String,
        params: Vec<Param>,
        reply: Reply<ResultHeader>,
    },
    Fetch {
        n: usize,
        reply: Reply<Fetched>,
    },
    Finish {
        reply: Reply<Completion>,
    },
    Explain {
        sql: String,
        reply: Reply<Option<u64>>,
    },
    UseDb {
        name: String,
        reply: Reply<()>,
    },
    Ping {
        reply: Reply<Duration>,
    },
    Close,
}

/// How an active result is discarded: Read results are killed (D28), others drained (D29).
struct Active {
    kill_on_finish: bool,
    cols: Arc<[Column]>,
    infos: Vec<ColInfo>,
}

pub(crate) async fn run(mut conn: Conn, opts: Opts, mut rx: mpsc::Receiver<ConnCmd>) {
    let cancel = MysqlCancel {
        opts,
        connection_id: conn.id(),
    };
    // Outcome of a result that ended by itself (exhausted), for a later `Finish`.
    let mut done: Result<Option<u64>, DriverError> = Ok(None);
    let mut next: Option<ConnCmd> = None;
    loop {
        let cmd = match next.take() {
            Some(c) => c,
            None => match rx.recv().await {
                Some(c) => c,
                None => break,
            },
        };
        match cmd {
            ConnCmd::Start { sql, mode, reply } => {
                done = Ok(None);
                match conn.query_iter(sql).await {
                    Ok(qr) => {
                        let read = mode == ExecMode::Read;
                        next = active(qr, read, reply, &mut rx, &cancel, &mut done).await;
                    }
                    Err(e) => {
                        let _ = reply.send(Err(map_err(e)));
                    }
                }
            }
            ConnCmd::StartParams { sql, params, reply } => {
                done = Ok(None);
                let values = params
                    .into_iter()
                    .map(|p| p.0.map_or(Value::NULL, |s| Value::Bytes(s.into_bytes())))
                    .collect();
                match conn.exec_iter(sql, Params::Positional(values)).await {
                    Ok(qr) => next = active(qr, false, reply, &mut rx, &cancel, &mut done).await,
                    Err(e) => {
                        let _ = reply.send(Err(map_err(e)));
                    }
                }
            }
            // Nothing active: an empty, exhausted result.
            ConnCmd::Fetch { reply, .. } => {
                let _ = reply.send(Ok(Fetched {
                    rows: Vec::new(),
                    exhausted: true,
                }));
            }
            ConnCmd::Finish { reply } => {
                let r = std::mem::replace(&mut done, Ok(None));
                let _ = reply.send(r.map(|affected_rows| Completion { affected_rows }));
            }
            ConnCmd::Explain { sql, reply } => {
                let _ = reply.send(explain(&mut conn, &sql).await);
            }
            ConnCmd::UseDb { name, reply } => {
                let q = format!("USE {}", MysqlDialect.quote_ident(&name));
                let _ = reply.send(conn.query_drop(q).await.map_err(map_err));
            }
            ConnCmd::Ping { reply } => {
                let t = std::time::Instant::now();
                let _ = reply.send(conn.ping().await.map(|()| t.elapsed()).map_err(map_err));
            }
            ConnCmd::Close => break,
        }
    }
    let _ = conn.disconnect().await;
}

fn header(cols: &[Column]) -> ResultHeader {
    ResultHeader {
        columns: cols
            .iter()
            .map(|c| ColumnMeta {
                name: c.name_str().into_owned(),
                type_name: type_name(c),
                category: ColInfo::of(c).category,
                nullable: nullable(c),
                is_primary_key: c.flags().contains(ColumnFlags::PRI_KEY_FLAG),
            })
            .collect(),
    }
}

/// Serves one result until it is exhausted, finished, failed or superseded. Returns the
/// command that superseded it (to run after the result was discarded).
async fn active<P: Protocol>(
    mut qr: QueryResult<'_, 'static, P>,
    read: bool,
    start_reply: Reply<ResultHeader>,
    rx: &mut mpsc::Receiver<ConnCmd>,
    cancel: &MysqlCancel,
    done: &mut Result<Option<u64>, DriverError>,
) -> Option<ConnCmd> {
    let cols = qr.columns().unwrap_or_else(|| Arc::from([]));
    let a = Active {
        kill_on_finish: read && !cols.is_empty(),
        infos: cols.iter().map(ColInfo::of).collect(),
        cols,
    };
    let _ = start_reply.send(Ok(header(&a.cols)));
    loop {
        // A dropped session closes the channel: go through the normal Close path (KILL/drain).
        match rx.recv().await.unwrap_or(ConnCmd::Close) {
            ConnCmd::Fetch { n, reply } => match fetch(&mut qr, &a.infos, n).await {
                Ok(f) => {
                    let exhausted = f.exhausted;
                    let _ = reply.send(Ok(f));
                    if exhausted {
                        *done = drain(&mut qr, a.cols.is_empty()).await.map_err(map_err);
                        return None;
                    }
                }
                Err(e) => {
                    let _ = reply.send(Err(map_err(e)));
                    return None;
                }
            },
            ConnCmd::Finish { reply } => {
                let r = finish(&mut qr, &a, cancel).await;
                let _ = reply.send(r.map(|affected_rows| Completion { affected_rows }));
                return None;
            }
            // D37: discard the result first; a failure there surfaces on the new command.
            other => {
                let _ = finish(&mut qr, &a, cancel).await;
                return Some(other);
            }
        }
    }
}

async fn fetch<P: Protocol>(
    qr: &mut QueryResult<'_, 'static, P>,
    infos: &[ColInfo],
    n: usize,
) -> mysql_async::Result<Fetched> {
    let mut rows: Vec<Row> = Vec::new();
    while rows.len() < n {
        let Some(row) = qr.next().await? else {
            return Ok(Fetched {
                rows,
                exhausted: true,
            });
        };
        rows.push(
            row.unwrap()
                .into_iter()
                .zip(infos)
                .map(|(v, i)| value_to_cell(v, i))
                .collect(),
        );
    }
    Ok(Fetched {
        rows,
        exhausted: false,
    })
}

/// Reads every remaining row and result set. Affected rows are those of the last OK packet,
/// reported only when the statement returned no rows.
async fn drain<P: Protocol>(
    qr: &mut QueryResult<'_, 'static, P>,
    no_rows: bool,
) -> mysql_async::Result<Option<u64>> {
    loop {
        while qr.next().await?.is_some() {}
        if qr.is_empty() {
            break;
        }
    }
    qr.next().await?; // clears a trailing row-less result set
    Ok(no_rows.then(|| qr.affected_rows()))
}

async fn finish<P: Protocol>(
    qr: &mut QueryResult<'_, 'static, P>,
    a: &Active,
    cancel: &MysqlCancel,
) -> Result<Option<u64>, DriverError> {
    if a.kill_on_finish {
        // Best effort: if the kill fails the drain below still ends the result.
        let _ = cancel.cancel().await;
    }
    match drain(qr, a.cols.is_empty()).await {
        // 1317 "interrupted" is the expected outcome of the kill.
        Err(e) => match map_err(e) {
            e if e.kind == crustdb_core::error::ErrorKind::Cancelled && a.kill_on_finish => {
                Ok(None)
            }
            e => Err(e),
        },
        Ok(n) => Ok(n),
    }
}

/// Largest `rows` estimate of the EXPLAIN plan for the statement's target table, else of
/// the whole plan.
async fn explain(conn: &mut Conn, sql: &str) -> Result<Option<u64>, DriverError> {
    let target = classify(sql, DriverKind::Mysql)
        .target
        .map(|t| t.name.to_lowercase());
    let plan: Vec<mysql_async::Row> = conn
        .query(format!("EXPLAIN {sql}"))
        .await
        .map_err(map_err)?;
    let est: Vec<(Option<String>, u64)> = plan
        .iter()
        .filter_map(|r| {
            let rows = r.get_opt::<Option<u64>, _>("rows")?.ok()??;
            let table = r
                .get_opt::<Option<String>, _>("table")
                .and_then(Result::ok)
                .flatten();
            Some((table, rows))
        })
        .collect();
    let of_target = est
        .iter()
        .filter(|(t, _)| t.as_ref().is_some_and(|t| Some(t.to_lowercase()) == target))
        .map(|(_, n)| *n)
        .max();
    Ok(of_target.or_else(|| est.iter().map(|(_, n)| *n).max()))
}
