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
}

pub struct Picker {
    tx: Sender<(Pick, PathBuf)>,
    rx: Receiver<(Pick, PathBuf)>,
}

impl Default for Picker {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self { tx, rx }
    }
}

impl Picker {
    /// Opens the dialog; a cancelled dialog sends nothing. `name` pre-fills the save name.
    pub fn start(&self, pick: Pick, ctx: &egui::Context, name: &str) {
        let (tx, ctx, name) = (self.tx.clone(), ctx.clone(), name.to_owned());
        std::thread::spawn(move || {
            let (label, ext) = match pick {
                Pick::Export(_) => ("CSV", "csv"),
                _ => ("SQL", "sql"),
            };
            let d = rfd::FileDialog::new().add_filter(label, &[ext]);
            let path = match pick {
                Pick::Open => d.pick_file(),
                _ => d.set_file_name(name).save_file(),
            };
            if let Some(p) = path {
                let _ = tx.send((pick, p));
                ctx.request_repaint();
            }
        });
    }

    pub fn poll(&self) -> Vec<(Pick, PathBuf)> {
        self.rx.try_iter().collect()
    }
}
