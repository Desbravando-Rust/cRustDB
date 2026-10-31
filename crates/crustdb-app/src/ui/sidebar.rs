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

/// Workspace selector, filter box and the lazy object tree of the active workspace.
pub fn show(ui: &mut egui::Ui, state: &AppState) -> Vec<Intent> {
    let mut out = vec![];
    let Some(active) = state.active_online() else {
        return out;
    };
    super::home::logo(ui);
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
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use crustdb_core::model::{Product, ServerVersion};

    use super::*;
    use crate::state::Status;
    use crate::state::workspace::tests::state;
    use crate::ui::home::tests::ready;

    #[test]
    fn sidebar_shows_logo_above_selector() {
        // Arrange
        let mut s = state(&["alpha"]);
        let ws = s.open_workspace(s.config.connections[0].clone());
        s.workspaces[0].status = Status::Online(ServerVersion {
            product: Product::Postgres,
            major: 16,
            minor: 0,
            patch: 0,
            raw: "16.0".into(),
        });
        s.active = Some(ws);
        let mut done = false;
        let mut h = Harness::builder().build_ui(|ui| {
            if ready(ui, &mut done) {
                show(ui, &s);
            }
        });

        // Act
        h.run();

        // Assert
        let logo = h.get_by_label("cRustDB logo").rect();
        let combo = h.get_by_role(egui::accesskit::Role::ComboBox).rect();
        assert!(logo.top() < combo.top());
    }

    fn online() -> AppState {
        let mut s = state(&["alpha"]);
        let ws = s.open_workspace(s.config.connections[0].clone());
        s.workspaces[0].status = Status::Online(ServerVersion {
            product: Product::Postgres,
            major: 16,
            minor: 0,
            patch: 0,
            raw: "16.0".into(),
        });
        s.active = Some(ws);
        s
    }

    #[test]
    fn sidebar_shows_label_chip() {
        // Arrange
        let mut s = online();
        s.config.connections[0].label = Some("dev".into());
        s.config.labels = vec![crate::storage::config::Label {
            name: "dev".into(),
            color: [1, 2, 3],
        }];
        let mut done = false;
        let mut h = Harness::builder().build_ui(|ui| {
            if ready(ui, &mut done) {
                show(ui, &s);
            }
        });

        // Act
        h.run();

        // Assert
        h.get_by_label("dev");
    }

    /// (shows "new", shows "dev") after applying `f`.
    fn chip_after(f: impl FnOnce(&mut AppState)) -> (bool, bool) {
        let mut s = online();
        s.workspaces[0].config.label = Some("dev".into());
        s.config.connections[0].label = Some("dev".into());
        f(&mut s);
        let mut done = false;
        let mut h = Harness::builder().build_ui(|ui| {
            if ready(ui, &mut done) {
                show(ui, &s);
            }
        });
        h.run();
        (
            h.query_by_label("new").is_some(),
            h.query_by_label("dev").is_some(),
        )
    }

    #[test]
    fn sidebar_chip_follows_label_rename() {
        // Arrange & Act
        let shown = chip_after(|s| s.config.connections[0].label = Some("new".into()));

        // Assert
        assert_eq!(shown, (true, false));
    }

    #[test]
    fn sidebar_chip_gone_after_label_delete() {
        // Arrange & Act
        let shown = chip_after(|s| s.config.connections[0].label = None);

        // Assert
        assert_eq!(shown, (false, false));
    }

    #[test]
    fn sidebar_logo_is_150_square() {
        // Arrange
        let s = online();
        let mut done = false;
        let mut h = Harness::builder().build_ui(|ui| {
            if ready(ui, &mut done) {
                show(ui, &s);
            }
        });

        // Act
        h.run();

        // Assert
        let size = h.get_by_label("cRustDB logo").rect().size();
        assert_eq!(size, egui::vec2(150.0, 150.0));
    }

    #[test]
    fn sidebar_buttons_are_spaced() {
        // Arrange
        let s = online();
        let mut done = false;
        let mut h = Harness::builder().build_ui(|ui| {
            if ready(ui, &mut done) {
                show(ui, &s);
            }
        });

        // Act
        h.run();

        // Assert
        let new = h.get_by_label("New query").rect();
        let disc = h.get_by_label("Disconnect").rect();
        assert!(disc.left() - new.right() >= 8.0 + h.ctx.global_style().spacing.item_spacing.x);
    }

    #[test]
    fn workspace_combo_has_accessible_name() {
        // Arrange
        let s = online();
        let mut done = false;
        let mut h = Harness::new_ui(|ui| {
            if ready(ui, &mut done) {
                show(ui, &s);
            }
        });

        // Act
        h.run();

        // Assert
        h.get_by_label("Workspace");
    }

    #[test]
    fn sidebar_filter_sits_in_gray_frame() {
        // Arrange
        let mut s = state(&["alpha"]);
        let ws = s.open_workspace(s.config.connections[0].clone());
        s.workspaces[0].status = Status::Online(ServerVersion {
            product: Product::Postgres,
            major: 16,
            minor: 0,
            patch: 0,
            raw: "16.0".into(),
        });
        s.active = Some(ws);
        let mut done = false;
        let mut h = Harness::builder().build_ui(|ui| {
            if ready(ui, &mut done) {
                show(ui, &s);
            }
        });

        // Act
        h.run();

        // Assert
        let input = h.get_by_role(egui::accesskit::Role::TextInput).rect();
        let framed = h.output().shapes.iter().any(|s| match &s.shape {
            egui::Shape::Rect(r) => {
                r.fill == egui::Color32::from_gray(40) && r.rect.contains_rect(input)
            }
            _ => false,
        });
        assert!(framed);
    }
}
