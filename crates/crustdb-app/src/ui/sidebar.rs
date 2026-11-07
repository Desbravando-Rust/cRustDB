use crustdb_core::model::DriverKind;
use eframe::egui;

use super::Intent;
use crate::state::AppState;
use crate::state::tree::NodePath;

/// Label of the active workspace, read from the saved connection (the workspace copy goes stale
/// after a catalog rename/delete).
fn active_label<'a>(
    state: &'a AppState,
    active: &'a crate::state::workspace::WorkspaceState,
) -> Option<&'a str> {
    state
        .config
        .connections
        .iter()
        .find(|c| c.id == active.config.id)
        .map_or(active.config.label.as_deref(), |c| c.label.as_deref())
}

/// App version at the bottom of the sidebar; draw it before the rest so it keeps its place.
pub fn footer(ui: &mut egui::Ui, state: &AppState) {
    egui::Panel::bottom("sidebar_footer").show(ui, |ui| {
        const VERSION: &str = env!("CARGO_PKG_VERSION");
        ui.weak(format!("v{VERSION}"));
        if let Some(tag) = state
            .news
            .latest
            .get()
            .filter(|t| crate::news::is_newer(VERSION, t))
        {
            ui.hyperlink_to(
                format!("Update available: {tag}"),
                crate::news::RELEASES_URL,
            );
        }
    });
}

/// Workspace selector, filter box and the lazy object tree of the active workspace.
pub fn show(ui: &mut egui::Ui, state: &AppState) -> Vec<Intent> {
    let mut out = vec![];
    let Some(active) = state.active_online() else {
        return out;
    };
    let label = active_label(state, active);
    let caps = active.capabilities();
    ui.horizontal(|ui| {
        if let Some(n) = label {
            let stripe = ui.max_rect().left_top();
            let c = super::labels::color32(super::labels::color_of(&state.config.labels, n));
            let bar = egui::Rect::from_min_size(stripe, egui::vec2(3.0, 22.0));
            ui.painter().rect_filled(bar, 1.0, c);
            ui.add_space(5.0);
        }
        let ws_combo = egui::ComboBox::from_id_salt("workspace")
            .selected_text(&active.config.name)
            .show_ui(ui, |ui| {
                for w in &state.workspaces {
                    if ui
                        .selectable_label(w.id == active.id, &w.config.name)
                        .clicked()
                    {
                        out.push(Intent::SwitchWorkspace(w.id));
                    }
                }
            });
        ws_combo.response.widget_info(|| {
            let mut i =
                egui::WidgetInfo::labeled(egui::Role::ComboBox, ui.is_enabled(), "Workspace");
            i.current_text_value = Some(active.config.name.clone());
            i
        });
        if let Some(n) = label {
            super::labels::chip(ui, n, &state.config.labels);
        }
    });
    ui.horizontal(|ui| {
        let new_query = ui
            .add_enabled(caps.sql_editor, egui::Button::new("New query"))
            .on_disabled_hover_text(super::home::unsupported_for(active.config.driver));
        if new_query.clicked() {
            out.push(Intent::NewQuery);
        }
        ui.add_space(8.0);
        if ui.button("Disconnect").clicked() {
            out.push(Intent::Disconnect(active.id));
        }
    });
    if let Some(ks) = &active.keyspace {
        out.extend(super::keyspace::sidebar(ui, ks));
        return out;
    }
    let mut q = active.tree.filter_text().to_owned();
    let edit = egui::Frame::new()
        .fill(egui::Color32::from_gray(40))
        .inner_margin(4.0)
        .show(ui, |ui| {
            ui.add(egui::TextEdit::singleline(&mut q).hint_text("filter"))
        });
    if edit.inner.changed() {
        out.push(Intent::Filter(q));
    }
    if let Some(e) = active.tree.root_error() {
        ui.colored_label(ui.visuals().error_fg_color, format!("Failed to load: {e}"));
        if ui.button("Retry").clicked() {
            out.push(Intent::RefreshRoot);
        }
    }
    egui::ScrollArea::both().show(ui, |ui| {
        for n in active.tree.visible() {
            ui.horizontal(|ui| {
                ui.add_space(n.depth as f32 * 14.0);
                match (&n.path, &n.object) {
                    (Some(path), _) => {
                        let arrow = if n.expanded { "▾" } else { "▸" };
                        let sel = active.tree.selected.as_ref() == Some(path);
                        let label = if n.loading {
                            format!("{arrow} {} …", n.label)
                        } else if n.error.is_some() {
                            format!("{arrow} {} (failed)", n.label)
                        } else {
                            format!("{arrow} {}", n.label)
                        };
                        let mut r = ui.selectable_label(sel, label);
                        if let Some(e) = &n.error {
                            r = r.on_hover_text(e);
                        }
                        if r.clicked() {
                            out.push(Intent::SelectNode(path.clone()));
                            out.push(Intent::ToggleNode(path.clone()));
                        }
                        let diagram = match path {
                            NodePath::Schema(db, s) => Some((db.clone(), Some(s.clone()))),
                            NodePath::Database(db) if active.config.driver == DriverKind::Mysql => {
                                Some((db.clone(), None))
                            }
                            _ => None,
                        };
                        if let Some((database, schema)) = diagram.filter(|_| caps.er_diagram) {
                            r.context_menu(|ui| {
                                if ui.button("ER diagram").clicked() {
                                    out.push(Intent::OpenDiagram { database, schema });
                                    ui.close();
                                }
                            });
                        }
                    }
                    (None, Some(object))
                        if ui.selectable_label(false, &n.label).double_clicked() =>
                    {
                        out.push(Intent::OpenObject {
                            object: object.clone(),
                        });
                    }
                    _ => {}
                }
            });
        }
    });
    out
}

#[cfg(test)]
#[path = "sidebar_tests.rs"]
mod tests;
