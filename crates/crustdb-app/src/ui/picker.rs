//! Native file dialogs (rfd, xdg-portal backend) run on a helper thread so frames never block.
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use eframe::egui;

use crate::runtime::TabId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Open,
    Save(TabId),
    Export(TabId),
    /// For the connection form whose draft has this id.
    SqliteDb(crustdb_core::model::ConnectionId),
}

pub struct Picker {
    tx: Sender<(Pick, Option<PathBuf>)>,
    rx: Receiver<(Pick, Option<PathBuf>)>,
    pending: bool,
}

impl Default for Picker {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            tx,
            rx,
            pending: false,
        }
    }
}

impl Picker {
    /// Claims the single dialog slot; false while another dialog is still open.
    // ponytail: one dialog app-wide; per-Pick Vec if users need parallel dialogs.
    // Known gap: a panic in the rfd thread leaves `pending` stuck.
    fn begin(&mut self) -> bool {
        !std::mem::replace(&mut self.pending, true)
    }

    /// Opens the dialog unless one is already open. `name` pre-fills the save name.
    pub fn start(&mut self, pick: Pick, ctx: &egui::Context, name: &str) {
        if !self.begin() {
            return;
        }
        let (tx, ctx, name) = (self.tx.clone(), ctx.clone(), name.to_owned());
        std::thread::spawn(move || {
            let d = rfd::FileDialog::new();
            // The export format follows the chosen extension (`export_fmt::Format::from_path`).
            let d = match pick {
                Pick::Export(_) => d
                    .add_filter("CSV", &["csv"])
                    .add_filter("JSON Lines", &["jsonl"])
                    .add_filter("SQL", &["sql"]),
                Pick::SqliteDb(_) => d
                    .add_filter("SQLite", &["db", "sqlite", "sqlite3"])
                    .add_filter("All files", &["*"]),
                Pick::Open | Pick::Save(_) => d.add_filter("SQL", &["sql"]),
            };
            let path = match pick {
                // The SQLite driver opens existing files only, so no save dialog here.
                Pick::Open | Pick::SqliteDb(_) => d.pick_file(),
                Pick::Save(_) | Pick::Export(_) => d.set_file_name(name).save_file(),
            };
            let _ = tx.send((pick, path));
            ctx.request_repaint();
        });
    }

    pub fn poll(&mut self) -> Vec<(Pick, PathBuf)> {
        let mut out = Vec::new();
        for (pick, path) in self.rx.try_iter() {
            self.pending = false;
            out.extend(path.map(|p| (pick, p)));
        }
        out
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn second_start_is_ignored_while_a_dialog_is_open() {
        // Arrange
        let mut picker = Picker::default();

        // Act
        let first = picker.begin();
        let second = picker.begin();

        // Assert
        assert!(first);
        assert!(!second);
    }

    #[test]
    fn cancelled_dialog_clears_pending() {
        // Arrange
        let mut picker = Picker::default();
        picker.begin();
        picker.tx.send((Pick::Open, None)).unwrap();

        // Act
        let polled = picker.poll();

        // Assert
        assert!(polled.is_empty());
        assert!(picker.begin());
    }

    #[test]
    fn picked_path_is_returned_and_clears_pending() {
        // Arrange
        let mut picker = Picker::default();
        picker.begin();
        let path = PathBuf::from("/tmp/a.sql");
        picker.tx.send((Pick::Open, Some(path.clone()))).unwrap();

        // Act
        let polled = picker.poll();

        // Assert
        assert_eq!(polled, vec![(Pick::Open, path)]);
        assert!(picker.begin());
    }

    #[test]
    fn poll_without_messages_keeps_pending() {
        // Arrange
        let mut picker = Picker::default();
        picker.begin();

        // Act
        let polled = picker.poll();

        // Assert
        assert!(polled.is_empty());
        assert!(!picker.begin());
    }
}
