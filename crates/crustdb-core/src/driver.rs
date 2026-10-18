use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::dialect::Dialect;
use crate::error::DriverError;
use crate::model::{
    ColumnDetail, ColumnMeta, Completion, DriverKind, ForeignKeyInfo, IndexInfo, ObjectKind,
    ObjectRef, Row, ServerVersion,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TlsMode {
    Disable,
    Require,
    VerifyCa,
    VerifyFull,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TlsConfig {
    pub mode: TlsMode,
    pub ca_path: Option<PathBuf>,
}

/// `host` is always the real host (TLS name); `connect_addr` is the tunnel address, if any.
#[derive(Clone)]
pub struct ConnectTarget {
    pub host: String,
    pub port: u16,
    pub connect_addr: Option<SocketAddr>,
    pub user: String,
    pub password: Option<SecretString>,
    pub database: Option<String>,
    pub tls: TlsConfig,
    pub connect_timeout: Duration,
}

impl fmt::Debug for ConnectTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectTarget")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("connect_addr", &self.connect_addr)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .field("database", &self.database)
            .field("tls", &self.tls)
            .field("connect_timeout", &self.connect_timeout)
            .finish()
    }
}

/// All purposes except `Editor` are read-only on the database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPurpose {
    Metadata,
    Data,
    Export,
    Editor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecMode {
    Read,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param(pub Option<String>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultHeader {
    pub columns: Vec<ColumnMeta>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Fetched {
    pub rows: Vec<Row>,
    pub exhausted: bool,
}

#[async_trait]
pub trait Driver: Send + Sync {
    fn kind(&self) -> DriverKind;
    fn dialect(&self) -> &dyn Dialect;
    fn introspector(&self) -> &dyn Introspector;
    async fn connect(
        &self,
        t: &ConnectTarget,
        p: SessionPurpose,
    ) -> Result<Box<dyn Session>, DriverError>;
}

#[async_trait]
pub trait Session: Send {
    fn server_version(&self) -> &ServerVersion;
    fn cancel_handle(&self) -> Box<dyn CancelHandle>;
    fn has_active_result(&self) -> bool;
    async fn ping(&mut self) -> Result<Duration, DriverError>;
    async fn start(&mut self, sql: &str, mode: ExecMode) -> Result<ResultHeader, DriverError>;
    async fn start_params(
        &mut self,
        sql: &str,
        params: &[Param],
    ) -> Result<ResultHeader, DriverError>;
    async fn fetch(&mut self, max_rows: usize) -> Result<Fetched, DriverError>;
    async fn finish(&mut self) -> Result<Completion, DriverError>;
    async fn explain_rows(&mut self, sql: &str) -> Result<Option<u64>, DriverError>;
    async fn use_database(&mut self, name: &str) -> Result<(), DriverError>;
    async fn close(self: Box<Self>);
}

#[async_trait]
pub trait CancelHandle: Send + Sync {
    async fn cancel(&self) -> Result<(), DriverError>;
}

#[async_trait]
pub trait Introspector: Send + Sync {
    async fn databases(&self, s: &mut dyn Session) -> Result<Vec<String>, DriverError>;
    async fn schemas(
        &self,
        s: &mut dyn Session,
        database: &str,
    ) -> Result<Vec<String>, DriverError>;
    async fn objects(
        &self,
        s: &mut dyn Session,
        database: &str,
        schema: Option<&str>,
        kind: ObjectKind,
    ) -> Result<Vec<ObjectRef>, DriverError>;
    async fn columns(
        &self,
        s: &mut dyn Session,
        o: &ObjectRef,
    ) -> Result<Vec<ColumnDetail>, DriverError>;
    async fn indexes(
        &self,
        s: &mut dyn Session,
        o: &ObjectRef,
    ) -> Result<Vec<IndexInfo>, DriverError>;
    async fn foreign_keys(
        &self,
        s: &mut dyn Session,
        o: &ObjectRef,
    ) -> Result<Vec<ForeignKeyInfo>, DriverError>;
    async fn ddl(&self, s: &mut dyn Session, o: &ObjectRef) -> Result<String, DriverError>;
    async fn row_estimate(
        &self,
        s: &mut dyn Session,
        o: &ObjectRef,
    ) -> Result<Option<u64>, DriverError>;
}
