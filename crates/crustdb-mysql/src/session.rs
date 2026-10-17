use std::time::{Duration, Instant};

use async_trait::async_trait;
use crustdb_core::driver::{CancelHandle, ExecMode, Fetched, Param, ResultHeader, Session};
use crustdb_core::error::{DriverError, ErrorKind};
use crustdb_core::model::{Completion, Product, ServerVersion};
use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Opts};

use crate::connect::{err, map_err};

pub struct MysqlSession {
    conn: Conn,
    opts: Opts,
    version: ServerVersion,
    affected: Option<u64>,
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
        Self {
            conn,
            opts,
            version: parse_version(raw_version),
            affected: None,
        }
    }
}

struct MysqlCancel {
    opts: Opts,
    connection_id: u32,
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
            connection_id: self.conn.id(),
        })
    }
    fn has_active_result(&self) -> bool {
        self.affected.is_some()
    }
    async fn ping(&mut self) -> Result<Duration, DriverError> {
        let t = Instant::now();
        self.conn.ping().await.map_err(map_err)?;
        Ok(t.elapsed())
    }
    // ponytail: minimal run-to-completion; the result-streaming task replaces this.
    async fn start(&mut self, sql: &str, _: ExecMode) -> Result<ResultHeader, DriverError> {
        self.affected = None;
        self.conn.query_drop(sql).await.map_err(map_err)?;
        self.affected = Some(self.conn.affected_rows());
        Ok(ResultHeader { columns: vec![] })
    }
    async fn start_params(&mut self, _: &str, _: &[Param]) -> Result<ResultHeader, DriverError> {
        Err(unsupported())
    }
    async fn fetch(&mut self, _: usize) -> Result<Fetched, DriverError> {
        Err(unsupported())
    }
    async fn finish(&mut self) -> Result<Completion, DriverError> {
        Ok(Completion {
            affected_rows: self.affected.take(),
        })
    }
    async fn explain_rows(&mut self, _: &str) -> Result<Option<u64>, DriverError> {
        Err(unsupported())
    }
    async fn use_database(&mut self, _: &str) -> Result<(), DriverError> {
        Err(unsupported())
    }
    async fn close(self: Box<Self>) {
        let _ = self.conn.disconnect().await;
    }
}

fn unsupported() -> DriverError {
    err(ErrorKind::Unsupported, "not implemented yet")
}
