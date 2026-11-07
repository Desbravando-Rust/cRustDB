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

/// Folder icon colour per object kind.
fn folder_color(kind: crustdb_core::model::ObjectKind) -> egui::Color32 {
    use crustdb_core::model::ObjectKind as K;
    match kind {
        K::Table => egui::Color32::from_rgb(96, 165, 250),
        K::View | K::MaterializedView => egui::Color32::from_rgb(167, 139, 250),
        K::Function | K::Procedure => egui::Color32::from_rgb(74, 222, 128),
        _ => egui::Color32::GRAY,
    }
}

/// Product name and app version at the bottom of the sidebar; draw it before the rest so it keeps its place.
pub fn footer(ui: &mut egui::Ui, state: &AppState) {
    egui::Panel::bottom("sidebar_footer").show(ui, |ui| {
        const VERSION: &str = env!("CARGO_PKG_VERSION");
        ui.horizontal(|ui| {
            ui.weak("cRustDB Desktop");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(format!("v{VERSION}"));
            });
        });
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
        ui.strong("CONNECTIONS");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let plus = ui.small_button(egui_phosphor::regular::PLUS);
            plus.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, true, "New connection")
            });
            if plus.clicked() {
                out.push(Intent::NewConnection);
            }
        });
    });
    for w in &state.workspaces {
        let is_active = w.id == active.id;
        let dot = if matches!(w.status, crate::state::Status::Online(_)) {
            egui::Color32::from_rgb(34, 197, 94)
        } else {
            egui::Color32::GRAY
        };
        let stroke = if is_active {
            egui::Stroke::new(1.0, super::theme::ACCENT)
        } else {
            egui::Stroke::NONE
        };
        let item = egui::Frame::new()
            .stroke(stroke)
            .corner_radius(4.0)
            .inner_margin(egui::Margin::symmetric(6, 2))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.colored_label(dot, egui_phosphor::regular::CIRCLE);
                    let r = ui.add(egui::Button::new(&w.config.name).frame(false));
                    if is_active && let Some(n) = label {
                        super::labels::chip(ui, n, &state.config.labels);
                    }
                    r
                })
                .inner
            });
        if item.inner.clicked() && !is_active {
            out.push(Intent::SwitchWorkspace(w.id));
        }
    }
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
    ui.strong("EXPLORER");
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
                        let arrow = if n.expanded {
                            egui_phosphor::regular::CARET_DOWN
                        } else {
                            egui_phosphor::regular::CARET_RIGHT
                        };
                        let sel = active.tree.selected.as_ref() == Some(path);
                        let label = if n.loading {
                            format!("{arrow} {} …", n.label)
                        } else if n.error.is_some() {
                            format!("{arrow} {} (failed)", n.label)
                        } else {
                            format!("{arrow} {}", n.label)
                        };
                        if let NodePath::Folder { kind, .. } = path {
                            ui.colored_label(folder_color(*kind), egui_phosphor::regular::FOLDER);
                        }
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
