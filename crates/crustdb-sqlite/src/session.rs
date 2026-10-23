use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use async_trait::async_trait;
use crustdb_core::driver::{CancelHandle, ExecMode, Fetched, Param, ResultHeader, Session};
use crustdb_core::error::{DriverError, ErrorKind};
use crustdb_core::model::{Completion, Product, ServerVersion};
use rusqlite::InterruptHandle;
use tokio::sync::{mpsc, oneshot};

use crate::conn_task::{ConnCmd, Reply};
use crate::connect::err;

pub struct SqliteSession {
    tx: mpsc::Sender<ConnCmd>,
    thread: JoinHandle<()>,
    interrupt: Arc<InterruptHandle>,
    version: ServerVersion,
    active: bool,
}

fn parse_version(raw: String) -> ServerVersion {
    // "3.46.0".
    let mut nums = raw.split('.').map(|p| p.parse().unwrap_or(0));
    let mut next = || nums.next().unwrap_or(0);
    ServerVersion {
        product: Product::Sqlite,
        major: next(),
        minor: next(),
        patch: next(),
        raw,
    }
}

impl SqliteSession {
    pub(crate) fn new(
        tx: mpsc::Sender<ConnCmd>,
        thread: JoinHandle<()>,
        interrupt: InterruptHandle,
        raw_version: String,
    ) -> Self {
        Self {
            tx,
            thread,
            interrupt: Arc::new(interrupt),
            version: parse_version(raw_version),
            active: false,
        }
    }

    /// D37: discards a paused result before a new command, reporting a failed drain.
    async fn settle(&mut self) -> Result<(), DriverError> {
        if self.active {
            self.finish().await?;
        }
        Ok(())
    }

    /// Sends a command and awaits its reply; a dead thread is a lost connection.
    async fn call<T>(&self, cmd: impl FnOnce(Reply<T>) -> ConnCmd) -> Result<T, DriverError> {
        let lost = || err(ErrorKind::Connect, "connection thread ended");
        let (reply, rx) = oneshot::channel();
        self.tx.send(cmd(reply)).await.map_err(|_| lost())?;
        rx.await.map_err(|_| lost())?
    }
}

/// `sqlite3_interrupt`: the running statement fails with `SQLITE_INTERRUPT` (Cancelled).
pub(crate) struct SqliteCancel(Arc<InterruptHandle>);

#[async_trait]
impl CancelHandle for SqliteCancel {
    async fn cancel(&self) -> Result<(), DriverError> {
        self.0.interrupt();
        Ok(())
    }
}

#[async_trait]
impl Session for SqliteSession {
    fn server_version(&self) -> &ServerVersion {
        &self.version
    }
    fn cancel_handle(&self) -> Box<dyn CancelHandle> {
        Box::new(SqliteCancel(Arc::clone(&self.interrupt)))
    }
    fn has_active_result(&self) -> bool {
        self.active
    }
    /// Finishes any active result first (D37), so a paused cursor is lost.
    async fn ping(&mut self) -> Result<Duration, DriverError> {
        self.settle().await?;
        self.call(|reply| ConnCmd::Ping { reply }).await
    }
    async fn start(&mut self, sql: &str, _mode: ExecMode) -> Result<ResultHeader, DriverError> {
        self.start_params(sql, &[]).await
    }
    async fn start_params(
        &mut self,
        sql: &str,
        params: &[Param],
    ) -> Result<ResultHeader, DriverError> {
        self.settle().await?;
        let (sql, params) = (sql.to_owned(), params.to_vec());
        let h = self
            .call(|reply| ConnCmd::Start { sql, params, reply })
            .await?;
        self.active = true;
        Ok(h)
    }
    async fn fetch(&mut self, max_rows: usize) -> Result<Fetched, DriverError> {
        let r = self
            .call(|reply| ConnCmd::Fetch { n: max_rows, reply })
            .await;
        if r.as_ref().map_or(true, |f| f.exhausted) {
            self.active = false;
        }
        r
    }
    async fn finish(&mut self) -> Result<Completion, DriverError> {
        self.active = false;
        self.call(|reply| ConnCmd::Finish { reply }).await
    }
    /// `EXPLAIN QUERY PLAN` has no row estimates: unknown (the strictest DML confirmation).
    async fn explain_rows(&mut self, _sql: &str) -> Result<Option<u64>, DriverError> {
        Ok(None)
    }
    /// Only `main` exists (no ATTACH support).
    async fn use_database(&mut self, name: &str) -> Result<(), DriverError> {
        self.settle().await?;
        if name == "main" {
            Ok(())
        } else {
            Err(err(
                ErrorKind::Unsupported,
                format!("SQLite has only the main database, not {name:?}"),
            ))
        }
    }
    async fn close(self: Box<Self>) {
        let Self { tx, thread, .. } = *self;
        let _ = tx.send(ConnCmd::Close).await;
        let _ = tokio::task::spawn_blocking(move || thread.join()).await;
    }
}
