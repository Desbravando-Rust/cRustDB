//! Command/Event protocol between the UI and the runtime.
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use crustdb_core::dialect::BuiltQuery;
use crustdb_core::driver::ResultHeader;
use crustdb_core::error::DriverError;
use crustdb_core::model::{
    ColumnDetail, ColumnMeta, Completion, ForeignKeyInfo, IndexInfo, ObjectKind, ObjectRef, Row,
    ServerVersion,
};
use crustdb_core::query::TableQuery;
use crustdb_core::safety::{Evaluation, Verdict};
pub use crustdb_core::sql::TxOp;
use crustdb_core::store::keyspace::EntryKey;
use crustdb_core::store::{EntryView, Reply, ScanPage};
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

/// One table of the ER diagram with what its box and edges need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagramTable {
    pub object: ObjectRef,
    pub columns: Vec<ColumnMeta>,
    pub fks: Vec<ForeignKeyInfo>,
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
    /// Column names of `object` for autocomplete, answered by `Event::Columns`.
    LoadColumns {
        ws: WorkspaceId,
        req: RequestId,
        object: ObjectRef,
    },
    /// Tables of one schema with columns and FKs, answered by `Event::Diagram`.
    LoadDiagram {
        ws: WorkspaceId,
        req: RequestId,
        database: String,
        schema: Option<String>,
    },
    /// Logical databases of a store workspace, answered by `Event::Namespaces`.
    ListNamespaces {
        ws: WorkspaceId,
        req: RequestId,
    },
    /// Selects another logical database of a store workspace: the sessions reconnect on it.
    SwitchNamespace {
        ws: WorkspaceId,
        ns: String,
    },
    /// One `ROWS_BATCH` of keys from `cursor` (`None` starts over), answered by `Event::KeysScanned`.
    ScanKeys {
        ws: WorkspaceId,
        req: RequestId,
        cursor: Option<String>,
        pattern: Option<String>,
    },
    /// One entry's value, answered by `Event::EntryRead`.
    ReadEntry {
        ws: WorkspaceId,
        req: RequestId,
        key: EntryKey,
    },
    /// Asks the driver how dangerous `argv` is, answered by `Event::CommandAssessed`.
    AssessCommand {
        ws: WorkspaceId,
        req: RequestId,
        argv: Vec<Vec<u8>>,
    },
    /// Runs `argv` on the editor session (one argument per element, never joined), answered
    /// by `Event::CommandReply`. Only sent after the assessment was approved.
    RunCommand {
        ws: WorkspaceId,
        req: RequestId,
        argv: Vec<Vec<u8>>,
    },
    /// Drops the store session while request `req` is in flight, or discards it while still
    /// queued; either way it answers "cancelled". Any other request is left alone.
    CancelStore {
        ws: WorkspaceId,
        req: RequestId,
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
    /// One-cell `UPDATE` from `edit::build_update`, run on the tab's own `Editor` session.
    UpdateCell {
        tab: TabId,
        req: RequestId,
        update: BuiltQuery,
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
    /// `None` when the lookup failed (so the caller may ask again).
    Columns {
        req: RequestId,
        columns: Option<Vec<String>>,
    },
    Children {
        req: RequestId,
        items: Vec<ObjectRef>,
    },
    /// `None` when the lookup failed.
    Diagram {
        req: RequestId,
        tables: Option<Vec<DiagramTable>>,
    },
    Namespaces {
        req: RequestId,
        result: Result<Vec<String>, String>,
    },
    KeysScanned {
        req: RequestId,
        result: Result<ScanPage, String>,
    },
    EntryRead {
        req: RequestId,
        result: Result<EntryView, String>,
    },
    /// `Err` is a refusal: the command must not run.
    CommandAssessed {
        req: RequestId,
        result: Result<(Verdict, String), String>,
    },
    CommandReply {
        req: RequestId,
        result: Result<Reply, String>,
    },
    Page {
        req: RequestId,
        header: ResultHeader,
        rows: Vec<Row>,
        has_more: bool,
        estimate: Option<u64>,
    },
    /// Outcome of the `UpdateCell` with this id: `None` changed a row (reload the page),
    /// `Some` is why it did not. Unlike `Notice`, it leaves pending page/structure loads alone.
    CellUpdated {
        req: RequestId,
        error: Option<String>,
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
    /// A Load more finished emitting its `Rows` batches.
    FetchDone {
        req: RequestId,
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
