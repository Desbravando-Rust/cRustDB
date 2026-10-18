//! Thin views: read the state, draw, and return intentions for `app.rs` to translate.
pub mod connection_form;
pub mod home;
pub mod sidebar;
pub mod theme;

use crustdb_core::model::{ConnectionId, ObjectRef};

use crate::runtime::WorkspaceId;
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
    OpenObject { object: ObjectRef },
}
