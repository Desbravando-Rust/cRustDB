//! Data tab toolbar: filter field, Advanced, round row actions and Commit Changes.
use eframe::egui::{self, RichText};
use egui_phosphor::regular as icon;

use super::Intent;
use super::grid::write_block;
use super::theme::ACCENT;
use crate::state::TableTab;

pub const FILTER_HINT: &str = "Filter rows…";
pub const EXPORT_LABEL: &str = "Export filtered rows...";
pub const ADVANCED_LABEL: &str = "Advanced";
pub const COMMIT_LABEL: &str = "Commit Changes";

/// Round, borderless-looking icon action.
fn round(icon: &str) -> egui::Button<'_> {
    egui::Button::new(icon)
        .corner_radius(egui::CornerRadius::same(255))
        .min_size(egui::vec2(28.0, 28.0))
}

pub fn show(ui: &mut egui::Ui, tab: &mut TableTab) -> Vec<Intent> {
    let mut out = vec![];
    let id = tab.id;
    ui.horizontal(|ui| {
        let text = format!("{} {}", icon::MAGNIFYING_GLASS, FILTER_HINT);
        let field = egui::Button::new(RichText::new(text).weak())
            .min_size(egui::vec2(220.0, 0.0))
            .fill(ui.visuals().extreme_bg_color);
        let field = ui.add(field);
        field.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, FILTER_HINT));
        if field.on_hover_text("Filter rows (Ctrl+F)").clicked() {
            tab.toggle_filter_bar();
        }
        let advanced = ui.add(egui::Button::new(format!(
            "{} {ADVANCED_LABEL}",
            icon::FUNNEL
        )));
        advanced
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, ADVANCED_LABEL));
        if advanced.clicked() {
            tab.toggle_filter_bar();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let on = tab.has_pending() && tab.edit_req.is_none();
            let mut commit = egui::Button::new(RichText::new(COMMIT_LABEL).strong());
            if on {
                commit = commit.fill(ACCENT);
            }
            if ui.add_enabled(on, commit).clicked() {
                // The grid is drawn after the toolbar: an open editor would be cancelled
                // this frame, so stage its text now.
                tab.commit_edit();
                out.push(Intent::SavePending(id));
            }
            // Right-to-left: export sits next to Commit, add row is the leftmost.
            let export = ui.add_enabled(tab.can_export(), round(icon::DOWNLOAD_SIMPLE));
            export
                .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, EXPORT_LABEL));
            if export.on_hover_text(EXPORT_LABEL).clicked() {
                out.push(Intent::ExportTable(id));
            }
            let reload = ui.add_enabled(!tab.is_loading(), round(icon::ARROW_CLOCKWISE));
            if reload.on_hover_text("Reload page (F5)").clicked() {
                out.push(Intent::Retry(id));
            }
            let block = write_block(tab);
            let add = ui.add_enabled(block.is_none(), round(icon::PLUS));
            if match block {
                Some(why) => add.on_disabled_hover_text(why),
                None => add.on_hover_text("Add row"),
            }
            .clicked()
            {
                out.push(Intent::AddRow(id));
            }
        });
    });
    out
}

#[cfg(test)]
#[path = "data_toolbar_tests.rs"]
mod tests;
