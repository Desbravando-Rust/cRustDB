//! In-memory `Driver` with programmable responses, for runtime and actor tests.
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
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
use tokio::sync::Notify;
use tokio::time::Instant;

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

/// What the fake observed and failure knobs; shared with every session it opens.
#[derive(Default)]
pub struct FakeLog {
    /// One entry per connect attempt (also failed ones): purpose, database, time.
    pub connects: Mutex<Vec<(SessionPurpose, Option<String>, Instant)>>,
    pub pings: AtomicUsize,
    pub cancels: AtomicUsize,
    /// The next N connect attempts fail.
    pub fail_connects: AtomicUsize,
    pub fail_pings: AtomicBool,
    /// The next query start panics (once).
    pub panic_next: AtomicBool,
    /// The next ping panics (once).
    pub panic_ping: AtomicBool,
    /// Every session call, in order: `start:<sql>`, `fetch`, `finish`, `explain:<sql>`,
    /// `estimate:<table>`, `use:<db>`.
    pub ops: Mutex<Vec<String>>,
    /// Statements containing the text fail with the error.
    pub fail_sql: Mutex<Vec<(String, DriverError)>>,
    /// The next statement start fails with a `Connect` error (dropped session), once.
    pub drop_next: AtomicBool,
    /// `explain_rows` fails.
    pub fail_explain: AtomicBool,
    /// `fetch` fails with a query error.
    pub fail_fetch: AtomicBool,
    /// Every `fetch` sleeps this many milliseconds (a cancel interrupts it).
    pub fetch_delay_ms: AtomicU64,
}

impl FakeLog {
    fn op(&self, s: String) {
        if let Ok(mut o) = self.ops.lock() {
            o.push(s);
        }
    }

    /// Snapshot of `ops`.
    pub fn ops(&self) -> Vec<String> {
        self.ops.lock().map(|o| o.clone()).unwrap_or_default()
    }
}

pub struct FakeDriver {
    pub kind: DriverKind,
    pub script: FakeScript,
    pub log: Arc<FakeLog>,
}

impl FakeDriver {
    pub fn new(script: FakeScript) -> Arc<Self> {
        Self::with_kind(script, DriverKind::Postgres)
    }

    pub fn with_kind(script: FakeScript, kind: DriverKind) -> Arc<Self> {
        Arc::new(Self {
            kind,
            script,
            log: Arc::default(),
        })
    }
}

struct FakeDialect(DriverKind);

impl Dialect for FakeDialect {
    fn kind(&self) -> DriverKind {
        self.0
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

static PG: FakeDialect = FakeDialect(DriverKind::Postgres);
static MYSQL: FakeDialect = FakeDialect(DriverKind::Mysql);

#[async_trait]
impl Driver for FakeDriver {
    fn kind(&self) -> DriverKind {
        self.kind
    }
    fn dialect(&self) -> &dyn Dialect {
        match self.kind {
            DriverKind::Postgres => &PG,
            DriverKind::Mysql => &MYSQL,
        }
    }
    fn introspector(&self) -> &dyn Introspector {
        self
    }
    async fn connect(
        &self,
        t: &ConnectTarget,
        p: SessionPurpose,
    ) -> Result<Box<dyn Session>, DriverError> {
        if let Ok(mut c) = self.log.connects.lock() {
            c.push((p, t.database.clone(), Instant::now()));
        }
        let failing = self
            .log
            .fail_connects
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok();
        if failing {
            return Err(DriverError {
                kind: ErrorKind::Connect,
                message: "fake connect failure".into(),
            });
        }
        Ok(Box::new(FakeSession {
            script: self.script.clone(),
            log: self.log.clone(),
            cancel: Arc::default(),
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
    log: Arc<FakeLog>,
    cancel: Arc<Notify>,
    version: ServerVersion,
    /// `Some(next row index)` while a result is active.
    pos: Option<usize>,
}

struct FakeCancel {
    log: Arc<FakeLog>,
    notify: Arc<Notify>,
}

#[async_trait]
impl CancelHandle for FakeCancel {
    async fn cancel(&self) -> Result<(), DriverError> {
        self.log.cancels.fetch_add(1, Ordering::SeqCst);
        self.notify.notify_one();
        Ok(())
    }
}

impl FakeSession {
    async fn begin(&mut self, sql: &str) -> Result<ResultHeader, DriverError> {
        self.log.op(format!("start:{sql}"));
        if self.log.drop_next.swap(false, Ordering::SeqCst) {
            return Err(DriverError {
                kind: ErrorKind::Connect,
                message: "fake connection lost".into(),
            });
        }
        let failing = self
            .log
            .fail_sql
            .lock()
            .ok()
            .and_then(|f| f.iter().find(|(p, _)| sql.contains(p.as_str())).cloned());
        if let Some((_, e)) = failing {
            return Err(e);
        }
        if self.log.panic_next.swap(false, Ordering::SeqCst) {
            panic!("fake driver panic");
        }
        // Drop a cancel permit left over from an earlier query.
        tokio::select! { biased; () = self.cancel.notified() => {}, () = std::future::ready(()) => {} }
        tokio::select! {
            () = tokio::time::sleep(self.script.delay) => {}
            () = self.cancel.notified() => {
                return Err(DriverError { kind: ErrorKind::Cancelled, message: "cancelled".into() });
            }
        }
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
        Box::new(FakeCancel {
            log: self.log.clone(),
            notify: self.cancel.clone(),
        })
    }
    fn has_active_result(&self) -> bool {
        self.pos.is_some()
    }
    async fn ping(&mut self) -> Result<Duration, DriverError> {
        if self.log.panic_ping.swap(false, Ordering::SeqCst) {
            panic!("fake ping panic");
        }
        self.log.pings.fetch_add(1, Ordering::SeqCst);
        if self.log.fail_pings.load(Ordering::SeqCst) {
            return Err(DriverError {
                kind: ErrorKind::Connect,
                message: "fake ping failure".into(),
            });
        }
        Ok(self.script.delay)
    }
    async fn start(&mut self, sql: &str, _: ExecMode) -> Result<ResultHeader, DriverError> {
        self.begin(sql).await
    }
    async fn start_params(&mut self, sql: &str, _: &[Param]) -> Result<ResultHeader, DriverError> {
        self.begin(sql).await
    }
    async fn fetch(&mut self, max_rows: usize) -> Result<Fetched, DriverError> {
        self.log.op("fetch".into());
        let delay = Duration::from_millis(self.log.fetch_delay_ms.load(Ordering::SeqCst));
        if !delay.is_zero() {
            tokio::select! {
                () = tokio::time::sleep(delay) => {}
                () = self.cancel.notified() => {
                    return Err(DriverError { kind: ErrorKind::Cancelled, message: "cancelled".into() });
                }
            }
        }
        if self.log.fail_fetch.load(Ordering::SeqCst) {
            return Err(DriverError {
                kind: ErrorKind::Query {
                    code: None,
                    position: None,
                },
                message: "fake fetch failure".into(),
            });
        }
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
        self.log.op("finish".into());
        self.pos = None;
        Ok(Completion {
            affected_rows: None,
        })
    }
    async fn explain_rows(&mut self, sql: &str) -> Result<Option<u64>, DriverError> {
        self.log.op(format!("explain:{sql}"));
        if self.log.fail_explain.load(Ordering::SeqCst) {
            return Err(DriverError {
                kind: ErrorKind::Query {
                    code: None,
                    position: None,
                },
                message: "fake explain failure".into(),
            });
        }
        Ok(self.script.estimate)
    }
    async fn use_database(&mut self, name: &str) -> Result<(), DriverError> {
        self.log.op(format!("use:{name}"));
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
        o: &ObjectRef,
    ) -> Result<Option<u64>, DriverError> {
        self.log.op(format!("estimate:{}", o.name));
        Ok(self.script.estimate)
    }
}
