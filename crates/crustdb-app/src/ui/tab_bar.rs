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
        )
    });
    ui.horizontal(|ui| {
        for (id, label) in tables.chain(queries) {
            if ui
                .selectable_label(w.active_tab == Some(id), label)
                .clicked()
            {
                out.push(Intent::ActivateTab(id));
            }
            if ui.small_button(egui_phosphor::regular::X).clicked() {
                out.push(Intent::CloseTab(id));
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
}
