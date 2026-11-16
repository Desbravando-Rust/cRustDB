//! Data tab footer: pager, row range, Refresh / Add row / Delete rows, pending edits.
use eframe::egui::{self, Color32};

use super::Intent;
use super::grid::{AMBER, read_only_reason, write_block};
use super::theme::ACCENT;
use crate::state::TableTab;
use crustdb_core::model::ObjectKind;

fn delete_button(ui: &mut egui::Ui, tab: &TableTab, out: &mut Vec<Intent>) {
    let block = write_block(tab).or(tab
        .selected_row
        .is_none()
        .then_some("Select rows to delete"));
    let r = ui.add_enabled(block.is_none(), egui::Button::new("Delete rows"));
    let r = match block {
        Some(why) => r.on_disabled_hover_text(why),
        None => r,
    };
    if r.clicked() {
        out.push(Intent::DeleteRows(tab.id));
    }
}

pub fn show(ui: &mut egui::Ui, tab: &TableTab) -> Vec<Intent> {
    let mut out = vec![];
    let id = tab.id;
    egui::Panel::bottom(("table_footer", id.0)).show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(tab.page > 0, egui::Button::new("<"))
                .clicked()
            {
                out.push(Intent::PrevPage(id));
            }
            if ui
                .add_enabled(tab.has_more, egui::Button::new(">"))
                .clicked()
            {
                out.push(Intent::NextPage(id));
            }
            ui.label(tab.footer_label());
            if tab.is_loading() && ui.button("Stop").clicked() {
                out.push(Intent::StopTab(id));
            }
            if ui
                .add_enabled(!tab.is_loading(), egui::Button::new("Refresh"))
                .clicked()
            {
                out.push(Intent::Retry(id));
            }
            let block = write_block(tab);
            let add = ui.add_enabled(block.is_none(), egui::Button::new("Add row"));
            if match block {
                Some(why) => add.on_disabled_hover_text(why),
                None => add,
            }
            .clicked()
            {
                out.push(Intent::AddRow(id));
            }
            delete_button(ui, tab, &mut out);
            if tab.object.kind == ObjectKind::Table
                && let Some(why) = read_only_reason(tab)
            {
                ui.colored_label(
                    Color32::YELLOW,
                    format!(
                        "{} Read-only: {}",
                        egui_phosphor::regular::WARNING,
                        why.to_lowercase()
                    ),
                );
            }
            if tab.has_pending() {
                egui::Frame::new()
                    .stroke(egui::Stroke::new(1.0, ACCENT))
                    .corner_radius(4.0)
                    .inner_margin(4.0)
                    .show(ui, |ui| {
                        if ui.button("Discard").clicked() {
                            out.push(Intent::DiscardPending(id));
                        }
                        match tab.pending.len() {
                            0 => ui.label("New row"),
                            1 => ui.label("1 change pending"),
                            n => ui.label(format!("{n} changes pending")),
                        };
                    });
            }
            if let Some(n) = tab.notice.clone() {
                ui.colored_label(ui.visuals().error_fg_color, n);
            }
            if tab.unordered() {
                ui.colored_label(AMBER, "unordered");
            }
        });
    });
    out
}
