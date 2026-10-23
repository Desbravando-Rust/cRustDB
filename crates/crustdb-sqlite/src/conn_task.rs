//! The thread that owns the `Connection`: a paused `Rows` borrows it, so it cannot live in the
//! session handle, and the SQLite C API blocks, so it cannot run on a tokio worker. Commands
//! arrive over a channel; while a result is active only Fetch/Finish are served and any other
//! command finishes it first (D37).
use std::time::{Duration, Instant};

use crustdb_core::driver::{Fetched, Param, ResultHeader};
use crustdb_core::error::DriverError;
use crustdb_core::model::{ColumnMeta, Completion, Row};
use rusqlite::{Connection, InterruptHandle, OpenFlags, Statement};
use tokio::sync::{mpsc, oneshot};

use crate::connect::map_err;
use crate::types::{category, value_to_cell};

pub(crate) type Reply<T> = oneshot::Sender<Result<T, DriverError>>;

/// What the session needs once the file is open: the cancel handle and `sqlite_version()`.
pub(crate) type Ready = (InterruptHandle, String);

pub(crate) enum ConnCmd {
    Start {
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
    Ping {
        reply: Reply<Duration>,
    },
    Close,
}

/// Reading `sqlite_schema` makes a non-database file fail here instead of on first use.
fn open(path: &str, flags: OpenFlags) -> Result<(Connection, String), DriverError> {
    let conn = Connection::open_with_flags(path, flags).map_err(map_err)?;
    conn.busy_timeout(Duration::from_secs(5)).map_err(map_err)?;
    let raw: String = conn
        .query_row(
            "SELECT sqlite_version(), (SELECT count(*) FROM sqlite_schema)",
            (),
            |r| r.get(0),
        )
        .map_err(map_err)?;
    Ok((conn, raw))
}

/// Runs on a dedicated `std::thread`, so `blocking_recv` never blocks the runtime.
pub(crate) fn run(
    path: String,
    flags: OpenFlags,
    ready: oneshot::Sender<Result<Ready, DriverError>>,
    mut rx: mpsc::Receiver<ConnCmd>,
) {
    let conn = match open(&path, flags) {
        Ok((conn, raw)) => {
            if ready.send(Ok((conn.get_interrupt_handle(), raw))).is_err() {
                return;
            }
            conn
        }
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    // Outcome of a row-less statement, for a later `Finish`.
    let mut done: Result<Option<u64>, DriverError> = Ok(None);
    let mut next: Option<ConnCmd> = None;
    loop {
        let cmd = match next.take() {
            Some(c) => c,
            None => match rx.blocking_recv() {
                Some(c) => c,
                None => break,
            },
        };
        match cmd {
            ConnCmd::Start { sql, params, reply } => {
                done = Ok(None);
                next = start(&conn, &sql, &params, reply, &mut rx, &mut done);
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
            ConnCmd::Ping { reply } => {
                let t = Instant::now();
                let r = conn.query_row("SELECT 1", (), |_| Ok(()));
                let _ = reply.send(r.map(|()| t.elapsed()).map_err(map_err));
            }
            ConnCmd::Close => break,
        }
    }
}

fn prepare<'c>(
    conn: &'c Connection,
    sql: &str,
    params: &[Param],
) -> rusqlite::Result<Statement<'c>> {
    let mut stmt = conn.prepare(sql)?;
    for (i, p) in params.iter().enumerate() {
        stmt.raw_bind_parameter(i + 1, p.0.as_deref())?;
    }
    Ok(stmt)
}

fn header(stmt: &Statement<'_>) -> ResultHeader {
    ResultHeader {
        columns: stmt
            .columns()
            .iter()
            .map(|c| ColumnMeta {
                name: c.name().to_owned(),
                type_name: c.decl_type().unwrap_or_default().to_owned(),
                category: category(c.decl_type()),
                nullable: None,
                is_primary_key: false,
            })
            .collect(),
    }
}

/// Runs a row-less statement at once; otherwise serves its rows until they are exhausted,
/// finished, failed or superseded. Returns the command that superseded the result (to run
/// after it was discarded by dropping `Rows`, which resets the statement).
fn start(
    conn: &Connection,
    sql: &str,
    params: &[Param],
    reply: Reply<ResultHeader>,
    rx: &mut mpsc::Receiver<ConnCmd>,
    done: &mut Result<Option<u64>, DriverError>,
) -> Option<ConnCmd> {
    let mut stmt = match prepare(conn, sql, params) {
        Ok(s) => s,
        Err(e) => {
            let _ = reply.send(Err(map_err(e)));
            return None;
        }
    };
    let ncols = stmt.column_count();
    if ncols == 0 {
        // `changes()` keeps the last DML count across DDL; an unchanged total means 0 rows.
        let before = conn.total_changes();
        match stmt.raw_execute() {
            Ok(_) => {
                let n = if conn.total_changes() == before {
                    0
                } else {
                    conn.changes()
                };
                *done = Ok(Some(n));
                let _ = reply.send(Ok(ResultHeader { columns: vec![] }));
            }
            Err(e) => {
                let _ = reply.send(Err(map_err(e)));
            }
        }
        return None;
    }
    let _ = reply.send(Ok(header(&stmt)));
    let mut rows = stmt.raw_query();
    loop {
        // A dropped session closes the channel: discard the result and stop.
        match rx.blocking_recv().unwrap_or(ConnCmd::Close) {
            ConnCmd::Fetch { n, reply } => match fetch(&mut rows, ncols, n) {
                Ok(f) => {
                    let exhausted = f.exhausted;
                    let _ = reply.send(Ok(f));
                    if exhausted {
                        return None;
                    }
                }
                Err(e) => {
                    let _ = reply.send(Err(map_err(e)));
                    return None;
                }
            },
            ConnCmd::Finish { reply } => {
                let _ = reply.send(Ok(Completion {
                    affected_rows: None,
                }));
                return None;
            }
            other => return Some(other),
        }
    }
}

fn fetch(rows: &mut rusqlite::Rows<'_>, ncols: usize, n: usize) -> rusqlite::Result<Fetched> {
    let mut out: Vec<Row> = Vec::new();
    while out.len() < n {
        let Some(r) = rows.next()? else {
            return Ok(Fetched {
                rows: out,
                exhausted: true,
            });
        };
        let row = (0..ncols)
            .map(|i| r.get_ref(i).map(value_to_cell))
            .collect::<rusqlite::Result<Row>>()?;
        out.push(row);
    }
    Ok(Fetched {
        rows: out,
        exhausted: false,
    })
}
