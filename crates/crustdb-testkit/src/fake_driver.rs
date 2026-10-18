//! In-memory `Driver` with programmable responses, for runtime and actor tests.
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use crustdb_core::dialect::Dialect;
use crustdb_core::driver::{
    CancelHandle, ConnectTarget, Driver, ExecMode, Fetched, Introspector, Param, ResultHeader,
    Session, SessionPurpose,
};
use crustdb_core::error::{DriverError, ErrorKind};
use crustdb_core::model::{
    ColumnDetail, ColumnMeta, Completion, DriverKind, ForeignKeyInfo, IndexInfo, ObjectKind,
    ObjectRef, Product, Row, ServerVersion,
};

#[derive(Clone)]
pub struct FakeScript {
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Row>,
    pub error: Option<DriverError>,
    pub delay: Duration,
    pub estimate: Option<u64>,
}

impl Default for FakeScript {
    fn default() -> Self {
        Self {
            columns: Vec::new(),
            rows: Vec::new(),
            error: None,
            delay: Duration::ZERO,
            estimate: None,
        }
    }
}

pub struct FakeDriver {
    pub script: FakeScript,
}

impl FakeDriver {
    pub fn new(script: FakeScript) -> Arc<Self> {
        Arc::new(Self { script })
    }
}

struct FakeDialect;

impl Dialect for FakeDialect {
    fn kind(&self) -> DriverKind {
        DriverKind::Postgres
    }
    fn quote_ident(&self, s: &str) -> String {
        format!("\"{}\"", s.replace('"', "\"\""))
    }
    fn qualified_name(&self, o: &ObjectRef) -> String {
        self.quote_ident(&o.name)
    }
    fn param(&self, index: usize, _: &ColumnMeta) -> String {
        format!("${index}")
    }
    fn select_expr(&self, c: &ColumnMeta) -> String {
        self.quote_ident(&c.name)
    }
    fn ci_like(&self, c: &ColumnMeta, param: &str) -> String {
        format!("{} ILIKE {param}", self.quote_ident(&c.name))
    }
    fn as_text(&self, c: &ColumnMeta) -> String {
        self.quote_ident(&c.name)
    }
}

static DIALECT: FakeDialect = FakeDialect;

#[async_trait]
impl Driver for FakeDriver {
    fn kind(&self) -> DriverKind {
        DriverKind::Postgres
    }
    fn dialect(&self) -> &dyn Dialect {
        &DIALECT
    }
    fn introspector(&self) -> &dyn Introspector {
        self
    }
    async fn connect(
        &self,
        _: &ConnectTarget,
        _: SessionPurpose,
    ) -> Result<Box<dyn Session>, DriverError> {
        Ok(Box::new(FakeSession {
            script: self.script.clone(),
            version: ServerVersion {
                product: Product::Postgres,
                major: 16,
                minor: 0,
                patch: 0,
                raw: "fake".into(),
            },
            pos: None,
        }))
    }
}

struct FakeSession {
    script: FakeScript,
    version: ServerVersion,
    /// `Some(next row index)` while a result is active.
    pos: Option<usize>,
}

struct FakeCancel;

#[async_trait]
impl CancelHandle for FakeCancel {
    async fn cancel(&self) -> Result<(), DriverError> {
        Ok(())
    }
}

impl FakeSession {
    async fn begin(&mut self) -> Result<ResultHeader, DriverError> {
        tokio::time::sleep(self.script.delay).await;
        if let Some(e) = &self.script.error {
            return Err(e.clone());
        }
        self.pos = Some(0);
        Ok(ResultHeader {
            columns: self.script.columns.clone(),
        })
    }
}

#[async_trait]
impl Session for FakeSession {
    fn server_version(&self) -> &ServerVersion {
        &self.version
    }
    fn cancel_handle(&self) -> Box<dyn CancelHandle> {
        Box::new(FakeCancel)
    }
    fn has_active_result(&self) -> bool {
        self.pos.is_some()
    }
    async fn ping(&mut self) -> Result<Duration, DriverError> {
        Ok(self.script.delay)
    }
    async fn start(&mut self, _: &str, _: ExecMode) -> Result<ResultHeader, DriverError> {
        self.begin().await
    }
    async fn start_params(&mut self, _: &str, _: &[Param]) -> Result<ResultHeader, DriverError> {
        self.begin().await
    }
    async fn fetch(&mut self, max_rows: usize) -> Result<Fetched, DriverError> {
        let start = self.pos.ok_or_else(|| DriverError {
            kind: ErrorKind::Protocol,
            message: "no active result".into(),
        })?;
        let end = (start + max_rows).min(self.script.rows.len());
        self.pos = Some(end);
        Ok(Fetched {
            rows: self.script.rows[start..end].to_vec(),
            exhausted: end == self.script.rows.len(),
        })
    }
    async fn finish(&mut self) -> Result<Completion, DriverError> {
        self.pos = None;
        Ok(Completion {
            affected_rows: None,
        })
    }
    async fn explain_rows(&mut self, _: &str) -> Result<Option<u64>, DriverError> {
        Ok(self.script.estimate)
    }
    async fn use_database(&mut self, _: &str) -> Result<(), DriverError> {
        Ok(())
    }
    async fn close(self: Box<Self>) {}
}

#[async_trait]
impl Introspector for FakeDriver {
    async fn databases(&self, _: &mut dyn Session) -> Result<Vec<String>, DriverError> {
        Ok(vec!["fake".into()])
    }
    async fn schemas(&self, _: &mut dyn Session, _: &str) -> Result<Vec<String>, DriverError> {
        Ok(vec!["public".into()])
    }
    async fn objects(
        &self,
        _: &mut dyn Session,
        _: &str,
        _: Option<&str>,
        _: ObjectKind,
    ) -> Result<Vec<ObjectRef>, DriverError> {
        Ok(Vec::new())
    }
    async fn columns(
        &self,
        _: &mut dyn Session,
        _: &ObjectRef,
    ) -> Result<Vec<ColumnDetail>, DriverError> {
        Ok(Vec::new())
    }
    async fn indexes(
        &self,
        _: &mut dyn Session,
        _: &ObjectRef,
    ) -> Result<Vec<IndexInfo>, DriverError> {
        Ok(Vec::new())
    }
    async fn foreign_keys(
        &self,
        _: &mut dyn Session,
        _: &ObjectRef,
    ) -> Result<Vec<ForeignKeyInfo>, DriverError> {
        Ok(Vec::new())
    }
    async fn ddl(&self, _: &mut dyn Session, _: &ObjectRef) -> Result<String, DriverError> {
        Ok("-- fake".into())
    }
    async fn row_estimate(
        &self,
        _: &mut dyn Session,
        _: &ObjectRef,
    ) -> Result<Option<u64>, DriverError> {
        Ok(self.script.estimate)
    }
}
