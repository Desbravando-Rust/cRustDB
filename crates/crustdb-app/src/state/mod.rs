//! UI-independent application state; everything here is testable without a GUI.
pub mod autocomplete;
pub mod connection_form;
pub mod diagram;
pub mod dialogs;
pub mod filter_draft;
pub mod query_tab;
pub mod result_pane;
pub mod status;
pub mod tabs;
pub mod tree;
pub mod workspace;

pub use dialogs::{Dialog, DialogStack};
pub use workspace::{AppState, Status, Tab, WorkspaceState};
pub mod table_tab;
pub use query_tab::QueryTab;
pub use table_tab::{Structure, TableTab, TableView};
