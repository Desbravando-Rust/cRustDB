//! Key tree (sidebar) and read-only value viewer (central panel) of a key-value workspace.
use std::time::Duration;

use eframe::egui;

use super::Intent;
use super::grid::grid;
use crate::state::keyspace::Keyspace;

const ROW_H: f32 = 18.0;

/// Namespaces, pattern filter (applied on Enter) and the virtualized key tree.
pub fn sidebar(ui: &mut egui::Ui, ks: &Keyspace) -> Vec<Intent> {
    let mut out = vec![];
    if !ks.namespaces.is_empty() {
        egui::CollapsingHeader::new(format!("Namespaces ({})", ks.namespaces.len())).show(
            ui,
            |ui| {
                for n in &ks.namespaces {
                    ui.weak(n);
                }
            },
        );
    }
    let id = ui.scope_id();
    let mut draft: String = ui.data_mut(|d| d.get_temp(id).unwrap_or_default());
    let r = ui.add(
        egui::TextEdit::singleline(&mut draft)
            .hint_text("pattern, e.g. user:*")
            .desired_width(f32::INFINITY),
    );
    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        out.push(Intent::KeyFilter(draft.clone()));
    }
    ui.data_mut(|d| d.insert_temp(id, draft));
    if let Some(e) = &ks.error {
        ui.colored_label(ui.visuals().error_fg_color, e);
    }
    let nodes = ks.nodes();
    let footer = ROW_H + 8.0;
    let height = (ui.available_height() - footer).max(ROW_H);
    egui::ScrollArea::vertical()
        .max_height(height)
        .auto_shrink([false, false])
        .show_rows(ui, ROW_H, nodes.len(), |ui, range| {
            for n in &nodes[range] {
                ui.horizontal(|ui| {
                    ui.add_space(n.depth as f32 * 14.0);
                    let arrow = match (n.branch, n.expanded) {
                        (false, _) => " ",
                        (true, true) => "▾",
                        (true, false) => "▸",
                    };
                    let sel = ks
                        .entry
                        .as_ref()
                        .is_some_and(|e| Some(&e.key) == n.key.as_ref());
                    if ui
                        .selectable_label(sel, format!("{arrow} {}", n.label))
                        .clicked()
                    {
                        if n.branch {
                            out.push(Intent::ToggleKey(n.path.clone()));
                        }
                        if let Some(k) = &n.key {
                            out.push(Intent::OpenKey(k.clone()));
                        }
                    }
                });
            }
        });
    ui.horizontal(|ui| {
        ui.weak(format!("{} keys", ks.scan.entries.len()));
        if !ks.scan.is_done() {
            let label = if ks.error.is_some() {
                "Retry"
            } else {
                "Load more"
            };
            if ui.button(label).clicked() {
                out.push(Intent::LoadMoreKeys);
            }
        }
    });
    out
}

fn ttl_text(ttl: Option<Duration>) -> String {
    ttl.map_or_else(|| "no expiry".into(), |d| format!("{}s", d.as_secs()))
}

/// Header (type, TTL, size, truncation) and the value rows of the opened key.
pub fn value_panel(ui: &mut egui::Ui, ks: &mut Keyspace) -> Vec<Intent> {
    let Some(e) = &mut ks.entry else {
        ui.weak("Select a key to view its value");
        return vec![];
    };
    ui.heading(e.key.display());
    ui.horizontal(|ui| {
        if let Some(k) = &e.kind {
            ui.label(format!("type: {k:?}"));
        }
        ui.label(format!("TTL: {}", ttl_text(e.ttl)));
        if let Some(b) = e.size_bytes {
            ui.label(format!("size: {b} B"));
        }
        if e.truncated {
            ui.colored_label(ui.visuals().warn_fg_color, "value truncated");
        }
    });
    ui.separator();
    grid(
        ui,
        &e.columns,
        &e.rows,
        None,
        &mut e.selected,
        &mut e.anchor,
        &[],
        None,
        &[],
        None,
    );
    vec![]
}
