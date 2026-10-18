//! Tab strip of the active workspace: table tabs, then query tabs.
use eframe::egui;

use super::Intent;
use crate::runtime::TxState;
use crate::state::{Tab, WorkspaceState};

pub fn show(ui: &mut egui::Ui, w: &WorkspaceState) -> Vec<Intent> {
    let mut out = vec![];
    let tables = w
        .tabs
        .iter()
        .map(|Tab::Table(t)| (t.id, t.object.name.clone()));
    let queries = w.queries.iter().enumerate().map(|(i, q)| {
        let name = q.file.as_ref().and_then(|f| f.file_name()).map_or_else(
            || format!("Query {}", i + 1),
            |n| n.to_string_lossy().into_owned(),
        );
        let mark = match (q.tx, q.dirty) {
            (TxState::Open, _) => " (tx)",
            (TxState::Aborted, _) => " (aborted)",
            (_, true) => " •",
            _ => "",
        };
        (q.id, format!("{name}{mark}"))
    });
    ui.horizontal(|ui| {
        for (id, label) in tables.chain(queries) {
            if ui
                .selectable_label(w.active_tab == Some(id), label)
                .clicked()
            {
                out.push(Intent::ActivateTab(id));
            }
            if ui.small_button(egui_phosphor::regular::X).clicked() {
                out.push(Intent::CloseTab(id));
            }
        }
    });
    out
}
