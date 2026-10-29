//! Thin views: read the state, draw, and return intentions for `app.rs` to translate.
pub mod autocomplete;
pub mod confirm_dialog;
pub mod connection_form;
pub mod diagram;
pub mod editor;
pub mod filter_bar;
pub mod find_bar;
pub mod grid;
pub mod grid_copy;
pub mod history_panel;
pub mod home;
pub mod picker;
pub mod results;
pub mod row_detail;
pub mod sidebar;
pub mod status_bar;
pub mod structure;
pub mod tab_bar;
pub mod theme;

use crustdb_core::model::{ConnectionId, ObjectRef};

use crate::runtime::{TabId, WorkspaceId};
use crate::state::tree::NodePath;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    Connect(ConnectionId),
    NewConnection,
    EditConnection(ConnectionId),
    DeleteConnection(ConnectionId),
    ConfirmDelete,
    CancelDialog,
    FormImportUrl,
    FormBrowseSqlite,
    FormTest,
    FormSave,
    FormConnect,
    FormCancel,
    SwitchWorkspace(WorkspaceId),
    Disconnect(WorkspaceId),
    ToggleNode(NodePath),
    SelectNode(NodePath),
    RefreshNode(NodePath),
    RefreshRoot,
    Filter(String),
    OpenObject {
        object: ObjectRef,
    },
    SortHeader {
        tab: TabId,
        column: String,
        shift: bool,
    },
    NextPage(TabId),
    PrevPage(TabId),
    StopTab(TabId),
    Retry(TabId),
    /// Stages the inline grid edit of the table tab.
    CommitEdit(TabId),
    /// Sends the staged row changes as one `UPDATE`.
    SavePending(TabId),
    DiscardPending(TabId),
    SetNull {
        tab: TabId,
        row: usize,
        col: usize,
    },
    ConfirmSafety,
    CloseResolve(crate::runtime::TxOp),
    DisconnectResolve(crate::runtime::TxOp),
    SaveFile,
    DiscardFile,
    AcceptHostKey,
    ApplyFilters(TabId),
    ClearFilters(TabId),
    SetView(TabId, crate::state::TableView),
    NewQuery,
    ActivateTab(TabId),
    CloseTab(TabId),
    /// `run` is the selection (or an empty range at the cursor), in chars.
    RunQuery {
        tab: TabId,
        run: std::ops::Range<usize>,
        all: bool,
    },
    Tx {
        tab: TabId,
        op: crate::runtime::TxOp,
    },
    UseDatabase {
        tab: TabId,
        database: String,
    },
    LoadMore(TabId),
    /// Native save dialog, then the export.
    ExportResult(TabId),
    OpenFile,
    Save(TabId),
    SaveAs(TabId),
    ToggleHistory,
    InsertHistory {
        tab: TabId,
        sql: String,
    },
    LoadColumns(ObjectRef),
    /// ER diagram of a schema (PG) or database (MySQL, `schema: None`).
    OpenDiagram {
        database: String,
        schema: Option<String>,
    },
}
