use crustdb_core::model::DriverKind;
use eframe::egui;

use super::Intent;
use crate::state::AppState;
use crate::state::tree::NodePath;

/// Workspace selector, filter box and the lazy object tree of the active workspace.
pub fn show(ui: &mut egui::Ui, state: &AppState) -> Vec<Intent> {
    let mut out = vec![];
    let Some(active) = state.active_online() else {
        return out;
    };
    egui::ComboBox::from_id_salt("workspace")
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
    if ui.button("New query").clicked() {
        out.push(Intent::NewQuery);
    }
    let mut q = active.tree.filter_text().to_owned();
    if ui
        .add(egui::TextEdit::singleline(&mut q).hint_text("filter"))
        .changed()
    {
        out.push(Intent::Filter(q));
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
                        } else {
                            format!("{arrow} {}", n.label)
                        };
                        let r = ui.selectable_label(sel, label);
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
                        if let Some((database, schema)) = diagram {
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
