//! In-memory `StoreDriver` with scripted pages, for actor tests of any key-value backend.
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use crustdb_core::driver::{ConnectTarget, SessionPurpose};
use crustdb_core::error::{DriverError, ErrorKind};
use crustdb_core::model::{DriverKind, Product, ServerVersion};
use crustdb_core::safety::Verdict;
use crustdb_core::store::keyspace::EntryKey;
use crustdb_core::store::{
    EntryView, Refused, Reply, ScanPage, ScanRequest, StoreDriver, StoreSession,
};

/// What the fake observed; shared with every session it opens.
#[derive(Default)]
pub struct FakeStoreLog {
    /// Every session call, in order: `namespaces`, `scan:<cursor|->:<count>`, `read:<key>:<limit>`.
    pub ops: Mutex<Vec<String>>,
    /// Connect attempts so far.
    pub connects: AtomicUsize,
    /// Sessions opened and not yet dropped.
    pub alive: AtomicUsize,
    /// `scan` never answers (until the session is dropped).
    pub hang_scan: AtomicBool,
    /// `run` never answers (until the session is dropped).
    pub hang_run: AtomicBool,
    /// Purpose of every connect, in order.
    pub purposes: Mutex<Vec<SessionPurpose>>,
    /// `database` of every connect target, in order.
    pub databases: Mutex<Vec<Option<String>>>,
}

impl FakeStoreLog {
    fn op(&self, s: String) {
        if let Ok(mut o) = self.ops.lock() {
            o.push(s);
        }
    }

    pub fn ops(&self) -> Vec<String> {
        self.ops.lock().map(|o| o.clone()).unwrap_or_default()
    }

    pub fn purposes(&self) -> Vec<SessionPurpose> {
        self.purposes.lock().map(|p| p.clone()).unwrap_or_default()
    }
}

#[derive(Clone)]
pub struct FakeStore {
    pub kind: DriverKind,
    /// `scan` answers `pages[n]` for cursor `Some("n")`, and `pages[0]` for no cursor.
    pub pages: Vec<ScanPage>,
    pub namespaces: Vec<String>,
    /// What `read` answers per key; other keys fail.
    pub entries: Vec<(EntryKey, EntryView)>,
    pub log: Arc<FakeStoreLog>,
}

impl FakeStore {
    pub fn new(kind: DriverKind) -> Self {
        Self {
            kind,
            pages: Vec::new(),
            namespaces: Vec::new(),
            entries: Vec::new(),
            log: Arc::default(),
        }
    }
}

#[async_trait]
impl StoreDriver for FakeStore {
    fn kind(&self) -> DriverKind {
        self.kind
    }

    /// FLUSHDB/FLUSHALL need the password, MULTI/SUBSCRIBE are refused, the rest runs.
    fn assess(&self, argv: &[Vec<u8>]) -> Result<(Verdict, String), Refused> {
        let name = argv.first().map(|a| a.to_ascii_uppercase());
        match name.as_deref() {
            Some(b"FLUSHDB" | b"FLUSHALL") => Ok((Verdict::Password, "can wipe data".into())),
            Some(b"MULTI" | b"SUBSCRIBE") => Err(Refused {
                reason: "not supported in this client".into(),
            }),
            _ => Ok((Verdict::Run, String::new())),
        }
    }

    async fn connect(
        &self,
        t: &ConnectTarget,
        p: SessionPurpose,
    ) -> Result<Box<dyn StoreSession>, DriverError> {
        if let Ok(mut v) = self.log.databases.lock() {
            v.push(t.database.clone());
        }
        if let Ok(mut v) = self.log.purposes.lock() {
            v.push(p);
        }
        self.log.connects.fetch_add(1, Ordering::SeqCst);
        self.log.alive.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FakeStoreSession {
            store: self.clone(),
            version: ServerVersion {
                product: Product::Redis,
                major: 7,
                minor: 0,
                patch: 0,
                raw: "fake-store".into(),
            },
        }))
    }
}

struct FakeStoreSession {
    store: FakeStore,
    version: ServerVersion,
}

impl Drop for FakeStoreSession {
    fn drop(&mut self) {
        self.store.log.alive.fetch_sub(1, Ordering::SeqCst);
    }
}

fn query_error(message: &str) -> DriverError {
    DriverError {
        kind: ErrorKind::Query {
            code: None,
            position: None,
        },
        message: message.into(),
    }
}

#[async_trait]
impl StoreSession for FakeStoreSession {
    fn server_version(&self) -> &ServerVersion {
        &self.version
    }

    async fn ping(&mut self) -> Result<Duration, DriverError> {
        Ok(Duration::ZERO)
    }

    async fn namespaces(&mut self) -> Result<Vec<String>, DriverError> {
        self.store.log.op("namespaces".into());
        Ok(self.store.namespaces.clone())
    }

    async fn scan(&mut self, req: &ScanRequest) -> Result<ScanPage, DriverError> {
        let cursor = req.cursor.as_deref();
        self.store
            .log
            .op(format!("scan:{}:{}", cursor.unwrap_or("-"), req.count));
        if self.store.log.hang_scan.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        let n = cursor.map_or(Some(0), |c| c.parse::<usize>().ok());
        n.and_then(|n| self.store.pages.get(n))
            .cloned()
            .ok_or_else(|| query_error("no such page"))
    }

    async fn read(&mut self, key: &EntryKey, limit: usize) -> Result<EntryView, DriverError> {
        self.store.log.op(format!("read:{}:{limit}", key.display()));
        self.store
            .entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .ok_or_else(|| query_error("no such key"))
    }

    /// Logs `run:<argc>` and answers the argument count.
    async fn run(&mut self, argv: &[Vec<u8>]) -> Result<Reply, DriverError> {
        self.store.log.op(format!("run:{}", argv.len()));
        if self.store.log.hang_run.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        Ok(Reply::Int(argv.len() as i64))
    }

    async fn close(self: Box<Self>) {}
}
