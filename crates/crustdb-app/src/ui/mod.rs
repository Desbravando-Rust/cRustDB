//! Thin views: read the state, draw, and return intentions for `app.rs` to translate.
pub mod confirm_dialog;
pub mod connection_form;
pub mod filter_bar;
pub mod grid;
pub mod home;
pub mod row_detail;
pub mod sidebar;
pub mod structure;
pub mod theme;

use crustdb_core::model::{ConnectionId, ObjectRef};

use crate::runtime::{TabId, WorkspaceId};
use crate::state::tree::NodePath;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    Connect(ConnectionId),
    NewConnection,
    EditConnection(ConnectionId),
    ImportUrl,
    DeleteConnection(ConnectionId),
    ConfirmDelete,
    CancelDialog,
    FormImportUrl,
    FormTest,
    FormSave,
    FormConnect,
    FormCancel,
    SwitchWorkspace(WorkspaceId),
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
    ConfirmSafety,
    CloseResolve(crate::runtime::TxOp),
    SaveFile,
    DiscardFile,
    AcceptHostKey,
    ApplyFilters(TabId),
    ClearFilters(TabId),
    SetView(TabId, crate::state::TableView),
}
