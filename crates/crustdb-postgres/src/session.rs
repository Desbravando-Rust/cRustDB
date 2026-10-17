use std::time::{Duration, Instant};

use async_trait::async_trait;
use crustdb_core::driver::{CancelHandle, ExecMode, Fetched, Param, ResultHeader, Session};
use crustdb_core::error::{DriverError, ErrorKind};
use crustdb_core::model::DriverKind;
use crustdb_core::model::{Completion, Product, ServerVersion, TypeCategory};
use crustdb_core::sql::{StatementClass, TxOp, classify};
use tokio_postgres::{Client, NoTls};
use tokio_postgres_rustls::MakeRustlsConnect;

use crate::connect::{err, map_err};
use crate::exec::Active;

pub struct PgSession {
    pub(crate) client: Client,
    version: ServerVersion,
    tls: Option<MakeRustlsConnect>,
    pub(crate) active: Option<Active>,
    pub(crate) cats: Vec<TypeCategory>,
    pub(crate) affected: Option<u64>,
    pub(crate) in_user_tx: bool,
}

fn parse_version(raw: String) -> ServerVersion {
    // "16.4 (Debian ...)", "17beta1": leading digits of each dotted part of the first token.
    let mut nums = raw
        .split_whitespace()
        .next()
        .unwrap_or("")
        .split('.')
        .map(|p| {
            p.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .unwrap_or(0)
        });
    let mut next = || nums.next().unwrap_or(0);
    ServerVersion {
        product: Product::Postgres,
        major: next(),
        minor: next(),
        patch: next(),
        raw,
    }
}

impl PgSession {
    pub(crate) fn new(client: Client, raw_version: String, tls: Option<MakeRustlsConnect>) -> Self {
        Self {
            client,
            version: parse_version(raw_version),
            tls,
            active: None,
            cats: Vec::new(),
            affected: None,
            in_user_tx: false,
        }
    }
}

struct PgCancel {
    token: tokio_postgres::CancelToken,
    tls: Option<MakeRustlsConnect>,
}

#[async_trait]
impl CancelHandle for PgCancel {
    async fn cancel(&self) -> Result<(), DriverError> {
        match &self.tls {
            Some(t) => self.token.cancel_query(t.clone()).await,
            None => self.token.cancel_query(NoTls).await,
        }
        .map_err(map_err)
    }
}

#[async_trait]
impl Session for PgSession {
    fn server_version(&self) -> &ServerVersion {
        &self.version
    }
    fn cancel_handle(&self) -> Box<dyn CancelHandle> {
        Box::new(PgCancel {
            token: self.client.cancel_token(),
            tls: self.tls.clone(),
        })
    }
    fn has_active_result(&self) -> bool {
        self.active.is_some()
    }
    async fn ping(&mut self) -> Result<Duration, DriverError> {
        let t = Instant::now();
        self.client
            .simple_query("SELECT 1")
            .await
            .map_err(map_err)?;
        Ok(t.elapsed())
    }
    async fn start(&mut self, sql: &str, mode: ExecMode) -> Result<ResultHeader, DriverError> {
        self.finish_active().await?;
        // D38: Describe for the column types; failure just means unknown types.
        // Inside a user tx a failed Describe aborts it, so report the real error now.
        let stmt = match self.client.prepare(sql).await {
            Ok(s) => Some(s),
            Err(e) if self.in_user_tx && e.as_db_error().is_some() => return Err(map_err(e)),
            Err(_) => None,
        };
        // Commit/Rollback end the transaction even when they fail.
        let tx = classify(sql, DriverKind::Postgres).class;
        if matches!(tx, StatementClass::Tx(TxOp::Commit | TxOp::Rollback)) {
            self.in_user_tx = false;
        }
        let columns = match mode {
            ExecMode::Read => self.start_cursor(sql, stmt.as_ref()).await?,
            ExecMode::Other => self.start_simple(sql, stmt.as_ref()).await?,
        };
        if tx == StatementClass::Tx(TxOp::Begin) {
            self.in_user_tx = true;
        }
        Ok(ResultHeader { columns })
    }
    async fn start_params(
        &mut self,
        sql: &str,
        params: &[Param],
    ) -> Result<ResultHeader, DriverError> {
        self.finish_active().await?;
        let columns = self.start_typed(sql, params).await?;
        Ok(ResultHeader { columns })
    }
    async fn fetch(&mut self, max_rows: usize) -> Result<Fetched, DriverError> {
        self.fetch_rows(max_rows.max(1)).await
    }
    async fn finish(&mut self) -> Result<Completion, DriverError> {
        self.finish_active().await
    }
    async fn explain_rows(&mut self, sql: &str) -> Result<Option<u64>, DriverError> {
        self.finish_active().await?;
        self.explain(sql).await
    }
    async fn use_database(&mut self, _: &str) -> Result<(), DriverError> {
        self.finish_active().await?;
        Err(err(
            ErrorKind::Unsupported,
            "PostgreSQL needs a new connection to switch database",
        ))
    }
    async fn close(self: Box<Self>) {}
}
