//! UI-independent application state; everything here is testable without a GUI.
pub mod autocomplete;
mod caps;
mod catalog;
#[cfg(test)]
mod catalog_tests;
pub mod command_pane;
pub mod connection_form;
pub mod diagram;
pub mod dialogs;
pub mod filter_draft;
pub mod find;
pub mod home_filter;
pub mod key_scan;
pub mod keyspace;
pub mod query_tab;
pub mod result_pane;
pub mod status;
pub mod tabs;
pub mod tree;
mod tree_filter;
mod tree_home;
pub mod workspace;
mod workspace_close;
mod workspace_fk;
mod workspace_rows;

pub use dialogs::{Dialog, DialogStack};
pub use workspace::{AppState, Status, Tab, WorkspaceState};
pub mod table_tab;
mod table_tab_edit;
pub use query_tab::QueryTab;
pub use table_tab::{Structure, TableTab, TableView};
