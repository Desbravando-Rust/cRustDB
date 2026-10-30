//! Central view of an online workspace with a configured database: its tables, double-click to open.
use eframe::egui;

use super::Intent;
use crate::state::WorkspaceState;

pub fn show(ui: &mut egui::Ui, w: &WorkspaceState) -> Vec<Intent> {
    let mut out = vec![];
    let Some(db) = &w.config.database else {
        return out;
    };
    ui.heading(db);
    ui.weak("Tables - double-click to open");
    let tables = w.tree.loaded_tables(db);
    if tables.is_empty() {
        ui.weak("No tables");
    }
    egui::ScrollArea::vertical().show(ui, |ui| {
        for t in tables {
            if ui.selectable_label(false, &t.name).double_clicked() {
                out.push(Intent::OpenObject { object: t.clone() });
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

    use crustdb_core::model::{ObjectKind, ObjectRef};

    use super::*;
    use crate::runtime::Command;
    use crate::state::workspace::tests::state;
    use crate::ui::home::tests::ready;

    fn table(name: &str) -> ObjectRef {
        ObjectRef {
            database: "d".into(),
            schema: Some("public".into()),
            name: name.into(),
            kind: ObjectKind::Table,
        }
    }

    fn loaded(names: &[&str]) -> WorkspaceState {
        let mut s = state(&[]);
        let mut c = crate::state::workspace::tests::conn("a");
        c.database = Some("d".into());
        let ws = s.open_workspace(c);
        let mut w = s.workspaces.remove(0);
        assert_eq!(w.id, ws);
        let cmds = w.tree.expand_tables_of("d");
        let Some(Command::LoadChildren { req, .. }) = cmds.last() else {
            panic!("no load command");
        };
        w.tree
            .apply_children(*req, names.iter().map(|n| table(n)).collect());
        w
    }

    #[test]
    fn workspace_home_lists_tables() {
        // Arrange
        let w = loaded(&["users", "orders"]);
        let mut done = false;
        let mut h = Harness::new_ui(|ui| {
            if ready(ui, &mut done) {
                show(ui, &w);
            }
        });

        // Act
        h.run();

        // Assert
        h.get_by_label("users");
        h.get_by_label("orders");
    }

    #[test]
    fn workspace_home_without_tables_says_so() {
        // Arrange
        let w = loaded(&[]);
        let mut done = false;
        let mut h = Harness::new_ui(|ui| {
            if ready(ui, &mut done) {
                show(ui, &w);
            }
        });

        // Act
        h.run();

        // Assert
        h.get_by_label("No tables");
    }

    #[test]
    fn workspace_home_double_click_opens_table() {
        // Arrange
        let w = loaded(&["users"]);
        let mut done = false;
        let mut got = vec![];
        // Short frames keep both clicks inside egui's double-click window.
        let mut h = Harness::builder().with_step_dt(0.01).build_ui(|ui| {
            if ready(ui, &mut done) {
                got.extend(show(ui, &w));
            }
        });
        h.run();

        // Act
        h.get_by_label("users").click();
        h.get_by_label("users").click();
        h.run();

        // Assert
        drop(h);
        assert!(got.contains(&Intent::OpenObject {
            object: table("users")
        }));
    }
}
