//! Data tab toolbar: filter field, add row, reload and Commit Changes.
use eframe::egui::{self, RichText};
use egui_phosphor::regular as icon;

use super::Intent;
use super::grid::write_block;
use super::theme::ACCENT;
use crate::state::TableTab;

pub const FILTER_HINT: &str = "Filter rows... (Ctrl+F)";
pub const COMMIT_LABEL: &str = "Commit Changes";

pub fn show(ui: &mut egui::Ui, tab: &mut TableTab) -> Vec<Intent> {
    let mut out = vec![];
    let id = tab.id;
    ui.horizontal(|ui| {
        let field = egui::Button::new(RichText::new(FILTER_HINT).weak())
            .min_size(egui::vec2(220.0, 0.0))
            .fill(ui.visuals().extreme_bg_color);
        if ui.add(field).clicked() {
            tab.toggle_filter_bar();
        }
        let block = write_block(tab);
        let add = ui.add_enabled(block.is_none(), egui::Button::new(icon::PLUS));
        if match block {
            Some(why) => add.on_disabled_hover_text(why),
            None => add.on_hover_text("Add row"),
        }
        .clicked()
        {
            out.push(Intent::AddRow(id));
        }
        let reload = ui.add_enabled(!tab.is_loading(), egui::Button::new(icon::ARROW_CLOCKWISE));
        if reload.on_hover_text("Reload page (F5)").clicked() {
            out.push(Intent::Retry(id));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let commit = egui::Button::new(RichText::new(COMMIT_LABEL).strong()).fill(ACCENT);
            let on = tab.has_pending() && tab.edit_req.is_none();
            if ui.add_enabled(on, commit).clicked() {
                // The grid is drawn after the toolbar: an open editor would be cancelled
                // this frame, so stage its text now.
                tab.commit_edit();
                out.push(Intent::SavePending(id));
            }
        });
    });
    out
}
