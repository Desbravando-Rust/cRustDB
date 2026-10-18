use std::time::Duration;

use async_trait::async_trait;
use crustdb_core::driver::{CancelHandle, ExecMode, Fetched, Param, ResultHeader, Session};
use crustdb_core::error::{DriverError, ErrorKind};
use crustdb_core::model::{Completion, Product, ServerVersion};
use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Opts};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::conn_task::{self, ConnCmd, Reply};
use crate::connect::{err, map_err};

pub struct MysqlSession {
    tx: mpsc::Sender<ConnCmd>,
    task: JoinHandle<()>,
    opts: Opts,
    connection_id: u32,
    version: ServerVersion,
    active: bool,
}

fn parse_version(raw: String) -> ServerVersion {
    // "8.4.3", "11.4.2-MariaDB-ubu2404".
    let product = if raw.contains("MariaDB") {
        Product::Mariadb
    } else {
        Product::Mysql
    };
    let mut nums = raw.split('.').map(|p| {
        p.chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .unwrap_or(0)
    });
    let mut next = || nums.next().unwrap_or(0);
    ServerVersion {
        product,
        major: next(),
        minor: next(),
        patch: next(),
        raw,
    }
}

impl MysqlSession {
    pub(crate) fn new(conn: Conn, opts: Opts, raw_version: String) -> Self {
        let connection_id = conn.id();
        let (tx, rx) = mpsc::channel(1);
        let task = tokio::spawn(conn_task::run(conn, opts.clone(), rx));
        Self {
            tx,
            task,
            opts,
            connection_id,
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

    /// Sends a command and awaits its reply; a dead task is a lost connection.
    async fn call<T>(&self, cmd: impl FnOnce(Reply<T>) -> ConnCmd) -> Result<T, DriverError> {
        let lost = || err(ErrorKind::Connect, "connection task ended");
        let (reply, rx) = oneshot::channel();
        self.tx.send(cmd(reply)).await.map_err(|_| lost())?;
        rx.await.map_err(|_| lost())?
    }

    /// 1317 is also what a shutting-down server sends before dropping the socket: with the task
    /// idle, a failed ping means the connection (and its transaction) is gone, not cancelled.
    async fn lost_if_dead<T>(&self, r: Result<T, DriverError>) -> Result<T, DriverError> {
        match r {
            Err(e) if e.kind == ErrorKind::Cancelled => {
                let alive = tokio::time::timeout(
                    Duration::from_secs(10),
                    self.call(|reply| ConnCmd::Ping { reply }),
                )
                .await;
                if matches!(alive, Ok(Ok(_))) {
                    Err(e)
                } else {
                    Err(err(ErrorKind::Connect, e.message))
                }
            }
            r => r,
        }
    }
}

pub(crate) struct MysqlCancel {
    pub(crate) opts: Opts,
    pub(crate) connection_id: u32,
}

#[async_trait]
impl CancelHandle for MysqlCancel {
    async fn cancel(&self) -> Result<(), DriverError> {
        let kill = async {
            let mut conn = Conn::new(self.opts.clone()).await.map_err(map_err)?;
            let r = conn
                .query_drop(format!("KILL QUERY {}", self.connection_id))
                .await
                .map_err(map_err);
            let _ = conn.disconnect().await;
            r
        };
        tokio::time::timeout(Duration::from_secs(10), kill)
            .await
            .map_err(|_| err(ErrorKind::Timeout, "cancel timed out"))?
    }
}

#[async_trait]
impl Session for MysqlSession {
    fn server_version(&self) -> &ServerVersion {
        &self.version
    }
    fn cancel_handle(&self) -> Box<dyn CancelHandle> {
        Box::new(MysqlCancel {
            opts: self.opts.clone(),
            connection_id: self.connection_id,
        })
    }
    fn has_active_result(&self) -> bool {
        self.active
    }
    /// Finishes any active result first (D37), so a paused cursor is lost.
    async fn ping(&mut self) -> Result<Duration, DriverError> {
        self.settle().await?;
        self.call(|reply| ConnCmd::Ping { reply }).await
    }
    async fn start(&mut self, sql: &str, mode: ExecMode) -> Result<ResultHeader, DriverError> {
        self.settle().await?;
        let sql = sql.to_owned();
        let r = self.call(|reply| ConnCmd::Start { sql, mode, reply }).await;
        let h = self.lost_if_dead(r).await?;
        self.active = true;
        Ok(h)
    }
    async fn start_params(
        &mut self,
        sql: &str,
        params: &[Param],
    ) -> Result<ResultHeader, DriverError> {
        self.settle().await?;
        let (sql, params) = (sql.to_owned(), params.to_vec());
        let r = self
            .call(|reply| ConnCmd::StartParams { sql, params, reply })
            .await;
        let h = self.lost_if_dead(r).await?;
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
        self.lost_if_dead(r).await
    }
    async fn finish(&mut self) -> Result<Completion, DriverError> {
        self.active = false;
        self.call(|reply| ConnCmd::Finish { reply }).await
    }
    async fn explain_rows(&mut self, sql: &str) -> Result<Option<u64>, DriverError> {
        self.settle().await?;
        let sql = sql.to_owned();
        self.call(|reply| ConnCmd::Explain { sql, reply }).await
    }
    async fn use_database(&mut self, name: &str) -> Result<(), DriverError> {
        self.settle().await?;
        let name = name.to_owned();
        self.call(|reply| ConnCmd::UseDb { name, reply }).await
    }
    async fn close(self: Box<Self>) {
        let _ = self.tx.send(ConnCmd::Close).await;
        let _ = self.task.await;
    }
}
