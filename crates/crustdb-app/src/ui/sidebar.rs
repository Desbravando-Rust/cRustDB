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

/// Small, grey, letter-spaced section title (`CONNECTIONS`, `EXPLORER`).
fn section_title(ui: &egui::Ui, text: &str) -> egui::RichText {
    egui::RichText::new(text)
        .small()
        .strong()
        .color(ui.visuals().weak_text_color())
        .extra_letter_spacing(1.5)
}

/// Decorative icon glyph: painted without a widget, so it has no accessible label.
fn icon(ui: &mut egui::Ui, glyph: &str) {
    let color = ui.visuals().text_color();
    paint_icon(ui, glyph, egui::FontId::proportional(14.0), color);
}

fn paint_icon(ui: &mut egui::Ui, glyph: &str, font: egui::FontId, color: egui::Color32) {
    let galley = ui.painter().layout_no_wrap(glyph.to_owned(), font, color);
    let (rect, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
    ui.painter().galley(rect.min, galley, color);
}

/// Icon of a leaf object row.
fn leaf_icon(kind: crustdb_core::model::ObjectKind) -> &'static str {
    use crustdb_core::model::ObjectKind as K;
    match kind {
        K::Table => egui_phosphor::regular::TABLE,
        K::View | K::MaterializedView => egui_phosphor::regular::EYE,
        _ => egui_phosphor::regular::FUNCTION,
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
    let caps = active.capabilities();
    ui.horizontal(|ui| {
        ui.label(section_title(ui, "CONNECTIONS"));
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
    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt("sidebar_connections")
        .max_height(160.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for w in &state.workspaces {
                let is_active = w.id == active.id;
                let online = matches!(w.status, crate::state::Status::Online(_));
                let wl = active_label(state, w);
                let item = egui::Frame::new()
                    .fill(if is_active {
                        super::theme::subtle_fill()
                    } else {
                        egui::Color32::TRANSPARENT
                    })
                    .corner_radius(4.0)
                    .inner_margin(egui::Margin::symmetric(6, 2))
                    .show(ui, |ui| {
                        // The whole row is the click and highlight target.
                        ui.set_width(ui.available_width());
                        super::theme::subtle_selection(ui.visuals_mut());
                        // Right edge first (dot, then chip); icon and name fill what is left.
                        ui.horizontal(|ui| {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let (rect, dot) = ui.allocate_exact_size(
                                    egui::vec2(8.0, 8.0),
                                    egui::Sense::hover(),
                                );
                                let (color, text) = if online {
                                    (super::theme::ONLINE, "Online")
                                } else {
                                    (egui::Color32::GRAY, "Offline")
                                };
                                ui.painter().circle_filled(rect.center(), 4.0, color);
                                dot.widget_info(|| {
                                    egui::WidgetInfo::labeled(egui::Role::Label, true, text)
                                });
                                dot.on_hover_text(text);
                                if let Some(n) = wl {
                                    super::labels::chip(ui, n, &state.config.labels);
                                }
                                ui.with_layout(
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        icon(ui, egui_phosphor::regular::DATABASE);
                                        // One widget: `grow` left-aligns the name while the
                                        // button spans the rest of the row (click, focus, a11y).
                                        ui.add(
                                            egui::Button::selectable(
                                                is_active,
                                                (w.config.name.as_str(), egui::Atom::grow()),
                                            )
                                            .frame(false)
                                            .truncate()
                                            .min_size(egui::vec2(ui.available_width(), 0.0)),
                                        )
                                    },
                                )
                                .inner
                            })
                            .inner
                        })
                        .inner
                    });
                if item.inner.clicked() && !is_active {
                    out.push(Intent::SwitchWorkspace(w.id));
                }
            }
        });
    if let Some(ks) = &active.keyspace {
        out.extend(super::keyspace::sidebar(ui, ks));
        return out;
    }
    ui.separator();
    ui.label(section_title(ui, "EXPLORER"));
    ui.separator();
    let mut q = active.tree.filter_text().to_owned();
    let edit = egui::Frame::new()
        .fill(super::theme::INPUT_FILL)
        .inner_margin(4.0)
        .show(ui, |ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut q).hint_text("filter"));
            super::a11y::name_text(&r, "Filter explorer");
            r
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
                            format!("{} …", n.label)
                        } else if n.error.is_some() {
                            format!("{} (failed)", n.label)
                        } else {
                            n.label.clone()
                        };
                        let caret = ui
                            .add(egui::Label::new(arrow).sense(egui::Sense::click()))
                            .on_hover_cursor(egui::CursorIcon::PointingHand);
                        caret.widget_info(|| {
                            let verb = if n.expanded { "Collapse" } else { "Expand" };
                            egui::WidgetInfo::labeled(
                                egui::Role::Button,
                                true,
                                format!("{verb} {}", n.label),
                            )
                        });
                        if let NodePath::Folder { kind, .. } = path {
                            paint_icon(
                                ui,
                                egui_phosphor::fill::FOLDER,
                                egui::FontId::new(14.0, super::theme::fill_family()),
                                folder_color(*kind),
                            );
                        }
                        let mut r = ui
                            .scope(|ui| {
                                super::theme::subtle_selection(ui.visuals_mut());
                                ui.selectable_label(sel, label)
                            })
                            .inner;
                        if let Some(e) = &n.error {
                            r = r.on_hover_text(e);
                        }
                        if r.clicked() || caret.clicked() {
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
                    (None, Some(object)) => {
                        // ponytail: O(folhas visíveis × abas) por frame; índice por objeto se árvores/abas crescerem
                        let open = active.tabs.iter().any(|t| t.object == *object);
                        icon(ui, leaf_icon(object.kind));
                        ui.scope(|ui| {
                            super::theme::subtle_selection(ui.visuals_mut());
                            if ui.selectable_label(open, &n.label).double_clicked() {
                                out.push(Intent::OpenObject {
                                    object: object.clone(),
                                });
                            }
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
#[path = "sidebar_nav_tests.rs"]
mod nav_tests;
#[cfg(test)]
#[path = "sidebar_tests.rs"]
mod tests;
