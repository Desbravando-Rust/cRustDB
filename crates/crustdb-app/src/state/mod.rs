//! UI-independent application state; everything here is testable without a GUI.
pub mod connection_form;
pub mod dialogs;
pub mod filter_draft;
pub mod tree;
pub mod workspace;

pub use dialogs::{Dialog, DialogStack};
pub use workspace::{AppState, Status, Tab, WorkspaceState};
pub mod table_tab;
pub use table_tab::{Structure, TableTab, TableView};
