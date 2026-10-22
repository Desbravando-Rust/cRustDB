//! Command/Event protocol between the UI and the runtime.
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use crustdb_core::driver::ResultHeader;
use crustdb_core::error::DriverError;
use crustdb_core::model::{
    ColumnDetail, Completion, ForeignKeyInfo, IndexInfo, ObjectKind, ObjectRef, Row, ServerVersion,
};
use crustdb_core::query::TableQuery;
use crustdb_core::safety::Evaluation;
pub use crustdb_core::sql::TxOp;
use secrecy::SecretString;

use crate::net::ssh::SshSecrets;
use crate::storage::config::ConnectionConfig;
use crate::storage::history::HistoryEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RequestId(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorkspaceId(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxState {
    Idle,
    Open,
    Aborted,
}

#[derive(Default)]
pub struct ResolvedSecrets {
    pub db: Option<SecretString>,
    pub ssh: SshSecrets,
}

impl fmt::Debug for ResolvedSecrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolvedSecrets")
            .field("db", &self.db.as_ref().map(|_| "***"))
            .field("ssh", &self.ssh)
            .finish()
    }
}

#[derive(Debug)]
pub enum Command {
    Connect {
        ws: WorkspaceId,
        config: ConnectionConfig,
        secrets: ResolvedSecrets,
    },
    Disconnect {
        ws: WorkspaceId,
    },
    HostKeyDecision {
        ws: WorkspaceId,
        accept: bool,
    },
    LoadChildren {
        ws: WorkspaceId,
        req: RequestId,
        database: String,
        schema: Option<String>,
        kind: Option<ObjectKind>,
    },
    OpenTable {
        ws: WorkspaceId,
        tab: TabId,
        object: ObjectRef,
    },
    FetchPage {
        tab: TabId,
        req: RequestId,
        query: TableQuery,
    },
    LoadStructure {
        tab: TabId,
        req: RequestId,
    },
    LoadDdl {
        tab: TabId,
        req: RequestId,
    },
    OpenEditor {
        ws: WorkspaceId,
        tab: TabId,
        database: Option<String>,
    },
    Evaluate {
        tab: TabId,
        req: RequestId,
        statements: Vec<String>,
    },
    Execute {
        tab: TabId,
        req: RequestId,
        evaluation: RequestId,
    },
    LoadMore {
        tab: TabId,
        req: RequestId,
    },
    Tx {
        tab: TabId,
        op: TxOp,
    },
    UseDatabase {
        tab: TabId,
        database: String,
    },
    CancelTab {
        tab: TabId,
    },
    CloseTab {
        tab: TabId,
        resolve: Option<TxOp>,
    },
    ExportTable {
        tab: TabId,
        req: RequestId,
        query: TableQuery,
        path: PathBuf,
    },
    ExportResult {
        tab: TabId,
        req: RequestId,
        path: PathBuf,
    },
    CancelExport {
        req: RequestId,
    },
}

#[derive(Debug)]
pub enum Event {
    Connected {
        ws: WorkspaceId,
        server: ServerVersion,
    },
    ConnectFailed {
        ws: WorkspaceId,
        error: String,
    },
    HostKeyPrompt {
        ws: WorkspaceId,
        host: String,
        fingerprint: String,
    },
    Children {
        req: RequestId,
        items: Vec<ObjectRef>,
    },
    Page {
        req: RequestId,
        header: ResultHeader,
        rows: Vec<Row>,
        has_more: bool,
        estimate: Option<u64>,
    },
    Structure {
        req: RequestId,
        columns: Vec<ColumnDetail>,
        indexes: Vec<IndexInfo>,
        fks: Vec<ForeignKeyInfo>,
    },
    Ddl {
        req: RequestId,
        text: String,
    },
    Evaluated {
        req: RequestId,
        evaluation: Evaluation,
    },
    StatementDone {
        req: RequestId,
        index: usize,
        completion: Completion,
        duration: Duration,
        history: HistoryEntry,
    },
    ResultStarted {
        req: RequestId,
        header: ResultHeader,
    },
    Rows {
        req: RequestId,
        rows: Vec<Row>,
        exhausted: bool,
    },
    ExecError {
        req: RequestId,
        index: usize,
        error: DriverError,
        history: HistoryEntry,
    },
    TxChanged {
        tab: TabId,
        state: TxState,
    },
    /// The Metadata session dropped; reconnecting with backoff. `Connected` follows on success.
    Reconnecting {
        ws: WorkspaceId,
    },
    Ping {
        ws: WorkspaceId,
        latency: Duration,
    },
    ExportProgress {
        req: RequestId,
        rows: u64,
    },
    ExportDone {
        req: RequestId,
        result: Result<u64, String>,
    },
    Notice {
        tab: TabId,
        text: String,
    },
    ActorCrashed {
        tab: TabId,
    },
}
