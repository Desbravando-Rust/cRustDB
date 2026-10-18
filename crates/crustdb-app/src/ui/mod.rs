//! Thin views: read the state, draw, and return intentions for `app.rs` to translate.
pub mod connection_form;
pub mod home;
pub mod theme;

use crustdb_core::model::ConnectionId;

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
}
