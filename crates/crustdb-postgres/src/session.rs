use std::time::{Duration, Instant};

use async_trait::async_trait;
use crustdb_core::driver::{CancelHandle, ExecMode, Fetched, Param, ResultHeader, Session};
use crustdb_core::error::{DriverError, ErrorKind};
use crustdb_core::model::{ColumnMeta, Completion, Product, ServerVersion, TypeCategory};
use tokio_postgres::{Client, NoTls, SimpleQueryMessage};
use tokio_postgres_rustls::MakeRustlsConnect;

use crate::connect::{err, map_err};

pub(crate) fn unsupported() -> DriverError {
    err(ErrorKind::Unsupported, "not implemented yet")
}

pub struct PgSession {
    client: Client,
    version: ServerVersion,
    tls: Option<MakeRustlsConnect>,
    affected: Option<u64>,
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
            affected: None,
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
        false
    }
    async fn ping(&mut self) -> Result<Duration, DriverError> {
        let t = Instant::now();
        self.client
            .simple_query("SELECT 1")
            .await
            .map_err(map_err)?;
        Ok(t.elapsed())
    }
    // ponytail: minimal placeholder, the execution task replaces it with real streaming.
    async fn start(&mut self, sql: &str, _mode: ExecMode) -> Result<ResultHeader, DriverError> {
        let mut columns = Vec::new();
        self.affected = None;
        for m in self.client.simple_query(sql).await.map_err(map_err)? {
            match m {
                SimpleQueryMessage::RowDescription(cols) if columns.is_empty() => {
                    columns = cols
                        .iter()
                        .map(|c| ColumnMeta {
                            name: c.name().to_string(),
                            type_name: String::new(),
                            category: TypeCategory::Other,
                            nullable: None,
                            is_primary_key: false,
                        })
                        .collect();
                }
                SimpleQueryMessage::CommandComplete(n) => self.affected = Some(n),
                _ => {}
            }
        }
        Ok(ResultHeader { columns })
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
    async fn close(self: Box<Self>) {}
}
