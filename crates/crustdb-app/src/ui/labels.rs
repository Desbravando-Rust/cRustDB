//! Connection labels: chips, list filter, form picker and catalog editor window.
use eframe::egui;

use super::Intent;
use crate::storage::config::{ConnectionConfig, Label};

/// One edit of the label catalog; the app applies it, propagates it and saves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelOp {
    Add(String),
    Rename {
        old: String,
        new: String,
    },
    /// In-memory only; `Persist` writes it once the drag ends.
    Color {
        name: String,
        color: [u8; 3],
    },
    Persist,
    Delete(String),
}

const FALLBACK: [u8; 3] = [128, 128, 128];

fn color32(c: [u8; 3]) -> egui::Color32 {
    egui::Color32::from_rgb(c[0], c[1], c[2])
}

fn color_of(labels: &[Label], name: &str) -> [u8; 3] {
    labels
        .iter()
        .find(|l| l.name == name)
        .map_or(FALLBACK, |l| l.color)
}

/// Readable text over the chip background.
fn text_on(c: [u8; 3]) -> egui::Color32 {
    let luma = 0.299 * f32::from(c[0]) + 0.587 * f32::from(c[1]) + 0.114 * f32::from(c[2]);
    if luma > 140.0 {
        egui::Color32::BLACK
    } else {
        egui::Color32::WHITE
    }
}

pub fn chip(ui: &mut egui::Ui, name: &str, labels: &[Label]) {
    let c = color_of(labels, name);
    ui.label(
        egui::RichText::new(name)
            .small()
            .color(text_on(c))
            .background_color(color32(c)),
    );
}

/// Case-insensitive substring match of the popup search over label names.
pub fn filter_options<'a>(labels: &'a [Label], query: &str) -> Vec<&'a Label> {
    let q = query.trim().to_lowercase();
    labels
        .iter()
        .filter(|l| l.name.to_lowercase().contains(&q))
        .collect()
}

pub fn matches(c: &ConnectionConfig, selected: Option<&str>) -> bool {
    selected.is_none_or(|s| c.labels.iter().any(|n| n == s))
}

/// Memory id of the selected filter label (`Option<String>`); session-only, never persisted.
pub(crate) fn filter_id() -> egui::Id {
    egui::Id::unique("label_filter")
}

/// Keeps the filter on a label that was renamed.
pub fn rename_filter(ctx: &egui::Context, old: &str, new: &str) {
    ctx.data_mut(|d| {
        let sel: Option<String> = d.get_temp(filter_id()).flatten();
        if sel.as_deref() == Some(old) {
            d.insert_temp(filter_id(), Some(new.to_string()));
        }
    });
}

/// Compact searchable select; returns the selected label name, if any.
pub fn filter_combo(ui: &mut egui::Ui, labels: &[Label]) -> Option<String> {
    let mut sel: Option<String> = ui.data(|d| d.get_temp(filter_id())).flatten();
    if sel
        .as_ref()
        .is_some_and(|s| !labels.iter().any(|l| &l.name == s))
    {
        sel = None;
    }
    let search_id = filter_id().with("search");
    let mut query: String = ui.data(|d| d.get_temp(search_id)).unwrap_or_default();
    egui::ComboBox::new("label_filter_combo", "Filter")
        .width(110.0)
        .selected_text(sel.as_deref().unwrap_or("All"))
        .show_ui(ui, |ui| {
            ui.add(egui::TextEdit::singleline(&mut query).hint_text("Search..."))
                .on_hover_text("Search labels");
            if ui.selectable_label(sel.is_none(), "All").clicked() {
                sel = None;
            }
            for l in filter_options(labels, &query) {
                let on = sel.as_deref() == Some(l.name.as_str());
                if ui.selectable_label(on, &l.name).clicked() {
                    sel = Some(l.name.clone());
                }
            }
        });
    ui.data_mut(|d| {
        d.insert_temp(filter_id(), sel.clone());
        d.insert_temp(search_id, query);
    });
    sel
}

/// Multi-select of catalog labels for the connection form.
pub fn picker(ui: &mut egui::Ui, selected: &mut Vec<String>, labels: &[Label]) {
    if labels.is_empty() && selected.is_empty() {
        return;
    }
    ui.label("Labels");
    // Names no longer in the catalog stay listed (checked) so they can be removed.
    let orphans: Vec<String> = selected
        .iter()
        .filter(|n| !labels.iter().any(|l| &l.name == *n))
        .cloned()
        .collect();
    let names = labels.iter().map(|l| &l.name).chain(orphans.iter());
    ui.horizontal_wrapped(|ui| {
        for name in names {
            let mut on = selected.contains(name);
            if ui.checkbox(&mut on, name).changed() {
                if on {
                    selected.push(name.clone());
                } else {
                    selected.retain(|n| n != name);
                }
            }
        }
    });
}

fn dirty_id() -> egui::Id {
    egui::Id::unique("labels_color_dirty")
}

fn open_id() -> egui::Id {
    egui::Id::unique("labels_window_open")
}

/// "Labels..." button plus the editor window; edits come back as intents.
pub fn editor(ui: &mut egui::Ui, labels: &[Label], read_only: bool) -> Vec<Intent> {
    let mut out = vec![];
    let ctx = ui.ctx().clone();
    let mut open: bool = ctx.data(|d| d.get_temp(open_id())).unwrap_or(false);
    if ui.button("Labels...").clicked() {
        open = true;
    }
    let mut keep = open;
    egui::Window::new("Labels")
        .open(&mut keep)
        .collapsible(false)
        .show(&ctx, |ui| {
            ui.add_enabled_ui(!read_only, |ui| rows(ui, labels, &mut out));
        });
    ctx.data_mut(|d| d.insert_temp(open_id(), keep));
    let dirty: bool = ctx.data(|d| d.get_temp(dirty_id())).unwrap_or(false);
    if dirty && !ctx.input(|i| i.pointer.any_down()) {
        ctx.data_mut(|d| d.insert_temp(dirty_id(), false));
        out.push(Intent::Label(LabelOp::Persist));
    }
    out
}

fn rows(ui: &mut egui::Ui, labels: &[Label], out: &mut Vec<Intent>) {
    for l in labels {
        ui.horizontal(|ui| {
            let id = egui::Id::unique(("label_name", &l.name));
            let mut buf: String = ui
                .data(|d| d.get_temp(id))
                .unwrap_or_else(|| l.name.clone());
            let r = ui
                .add(egui::TextEdit::singleline(&mut buf).desired_width(120.0))
                .on_hover_text(format!("Name of label {}", l.name));
            if r.lost_focus() {
                ui.data_mut(|d| d.remove_temp::<String>(id));
                if buf != l.name {
                    let op = LabelOp::Rename {
                        old: l.name.clone(),
                        new: buf,
                    };
                    out.push(Intent::Label(op));
                }
            } else {
                ui.data_mut(|d| d.insert_temp(id, buf));
            }
            let mut c = l.color;
            let cr = ui
                .color_edit_button_srgb(&mut c)
                .on_hover_text(format!("Color of {}", l.name));
            if cr.changed() {
                ui.data_mut(|d| d.insert_temp(dirty_id(), true));
                let op = LabelOp::Color {
                    name: l.name.clone(),
                    color: c,
                };
                out.push(Intent::Label(op));
            }
            if ui
                .button("Delete")
                .on_hover_text(format!("Delete label {}", l.name))
                .clicked()
            {
                out.push(Intent::Label(LabelOp::Delete(l.name.clone())));
            }
        });
    }
    ui.horizontal(|ui| {
        let id = egui::Id::unique("label_new");
        let mut buf: String = ui.data(|d| d.get_temp(id)).unwrap_or_default();
        ui.add(
            egui::TextEdit::singleline(&mut buf)
                .hint_text("New label")
                .desired_width(120.0),
        );
        if ui.button("Add").clicked() {
            out.push(Intent::Label(LabelOp::Add(std::mem::take(&mut buf))));
        }
        ui.data_mut(|d| d.insert_temp(id, buf));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lab(n: &str) -> Label {
        Label {
            name: n.into(),
            color: [0, 0, 0],
        }
    }

    #[test]
    fn home_filter_search_narrows_options() {
        // Arrange
        let cat = [lab("local"), lab("Production"), lab("staging")];

        // Act
        let names: Vec<_> = filter_options(&cat, " PROD ")
            .iter()
            .map(|l| l.name.as_str())
            .collect();

        // Assert
        assert_eq!(names, ["Production"]);
        assert_eq!(filter_options(&cat, "").len(), 3);
    }
}
