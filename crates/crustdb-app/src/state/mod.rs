//! UI-independent application state; everything here is testable without a GUI.
pub mod dialogs;
pub mod workspace;

pub use dialogs::{Dialog, DialogStack};
pub use workspace::{AppState, Status, WorkspaceState};
