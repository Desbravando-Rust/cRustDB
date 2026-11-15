//! Tab strip of the active workspace: table tabs, then query tabs.
use eframe::egui;

use super::Intent;
use crate::runtime::TxState;
use crate::state::{Tab, WorkspaceState};

pub fn show(ui: &mut egui::Ui, w: &WorkspaceState) -> Vec<Intent> {
    let mut out = vec![];
    let tables = w.tabs.iter().map(|Tab::Table(t)| {
        (
            t.id,
            format!("{} Data: {}", egui_phosphor::regular::TABLE, t.object.name),
            t.object.schema.as_ref().map_or_else(
                || t.object.name.clone(),
                |sc| format!("{sc}.{}", t.object.name),
            ),
        )
    });
    let queries = w.queries.iter().enumerate().map(|(i, q)| {
        let name = q.file.as_ref().and_then(|f| f.file_name()).map_or_else(
            || format!("Query {}", i + 1),
            |n| n.to_string_lossy().into_owned(),
        );
        let mark = match (q.tx, q.dirty) {
            (TxState::Open, _) => " (tx)",
            (TxState::Aborted, _) => " (aborted)",
            (_, true) => " •",
            _ => "",
        };
        (
            q.id,
            format!("{} {name}{mark}", egui_phosphor::regular::FILE_SQL),
            name,
        )
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (id, label, name) in tables.chain(queries) {
            let active = w.active_tab == Some(id);
            let tab = egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(6, 2))
                .show(ui, |ui| {
                    super::theme::subtle_selection(ui.visuals_mut());
                    let r =
                        ui.add(egui::Button::selectable(active, &label).frame_when_inactive(false));
                    let x = ui.add(
                        egui::Button::new(egui_phosphor::regular::X)
                            .small()
                            .frame(false),
                    );
                    x.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::Role::Button,
                            true,
                            format!("Close tab {name}"),
                        )
                    });
                    if r.clicked() {
                        out.push(Intent::ActivateTab(id));
                    }
                    if x.clicked() {
                        out.push(Intent::CloseTab(id));
                    }
                });
            if active {
                let rect = tab.response.rect;
                ui.painter().hline(
                    rect.x_range(),
                    rect.bottom(),
                    egui::Stroke::new(2.0, super::theme::ACCENT),
                );
            }
        }
    });
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;
    use crate::runtime::TabId;
    use crate::state::TableTab;
    use crate::state::workspace::tests::state;
    use crustdb_core::model::{ObjectKind, ObjectRef};

    #[test]
    fn tab_bar_table_tab_reads_data_prefix() {
        // Arrange
        let mut s = state(&["alpha"]);
        let ws = s.open_workspace(s.config.connections[0].clone());
        let object = ObjectRef {
            database: "d".into(),
            schema: None,
            name: "users".into(),
            kind: ObjectKind::Table,
        };
        let w = s.workspace_mut(ws).unwrap();
        w.tabs
            .push(Tab::Table(TableTab::new(TabId(1), object, 100)));
        let mut done = false;
        let mut h = Harness::new_ui(|ui| {
            if crate::ui::home::tests::ready(ui, &mut done) {
                show(ui, s.workspaces.first().unwrap());
            }
        });

        // Act
        h.run();

        // Assert
        assert!(h.query_by_label_contains("Data: users").is_some());
    }

    fn two_tabs() -> crate::state::AppState {
        let mut s = state(&["alpha"]);
        let ws = s.open_workspace(s.config.connections[0].clone());
        let object = |n: &str| ObjectRef {
            database: "d".into(),
            schema: None,
            name: n.into(),
            kind: ObjectKind::Table,
        };
        let w = s.workspace_mut(ws).unwrap();
        w.tabs
            .push(Tab::Table(TableTab::new(TabId(1), object("users"), 100)));
        w.tabs
            .push(Tab::Table(TableTab::new(TabId(2), object("orders"), 100)));
        w.active_tab = Some(TabId(1));
        s
    }

    #[test]
    fn tab_bar_close_button_is_labeled_and_sits_inside_the_tab() {
        // Arrange
        let s = two_tabs();
        let mut done = false;
        let mut got = vec![];
        let mut h = Harness::new_ui(|ui| {
            if crate::ui::home::tests::ready(ui, &mut done) {
                got.extend(show(ui, s.workspaces.first().unwrap()));
            }
        });
        h.run();
        let tab = h.get_by_label_contains("Data: users").rect();
        let next = h.get_by_label_contains("Data: orders").rect();
        let close = h.get_by_label("Close tab users").rect();

        // Act
        h.get_by_label("Close tab users").click();
        h.run();

        // Assert
        assert!(close.left() >= tab.right() - 1.0, "X follows its tab label");
        assert!(close.right() <= next.left(), "X precedes the next tab");
        assert!(close.center().y > tab.top() && close.center().y < tab.bottom());
        drop(h);
        assert!(got.contains(&Intent::CloseTab(TabId(1))));
    }

    #[test]
    fn tab_bar_close_labels_tell_same_named_tables_apart() {
        // Arrange
        let mut s = state(&["alpha"]);
        let ws = s.open_workspace(s.config.connections[0].clone());
        let w = s.workspace_mut(ws).unwrap();
        for (id, schema) in [(1, "public"), (2, "audit")] {
            let object = ObjectRef {
                database: "d".into(),
                schema: Some(schema.into()),
                name: "users".into(),
                kind: ObjectKind::Table,
            };
            w.tabs
                .push(Tab::Table(TableTab::new(TabId(id), object, 100)));
        }
        let mut done = false;
        let mut got = vec![];
        let mut h = Harness::new_ui(|ui| {
            if crate::ui::home::tests::ready(ui, &mut done) {
                got.extend(show(ui, s.workspaces.first().unwrap()));
            }
        });
        h.run();

        // Act
        h.get_by_label("Close tab audit.users").click();
        h.run();

        // Assert
        h.get_by_label("Close tab public.users");
        drop(h);
        assert!(got.contains(&Intent::CloseTab(TabId(2))));
    }
}
