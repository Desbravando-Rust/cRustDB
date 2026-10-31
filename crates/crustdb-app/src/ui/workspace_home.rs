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
    if !w.capabilities().table_tab {
        ui.weak("No tables in this database");
        return out;
    }
    ui.weak("Tables - double-click to open");
    if let Some(msg) = w.tree.tables_error(db) {
        ui.colored_label(
            ui.visuals().error_fg_color,
            format!("Failed to load tables: {msg}"),
        );
        if ui.button("Retry").clicked() {
            out.push(Intent::RefreshNode(w.tree.home_tables_node(db)));
        }
        return out;
    }
    let Some(tables) = w.tree.loaded_tables(db) else {
        ui.label("Loading…");
        return out;
    };
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

    /// Workspace whose tables folder load was requested but not answered yet.
    fn pending() -> (WorkspaceState, crate::runtime::RequestId) {
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
        let req = *req;
        (w, req)
    }

    fn loaded(names: &[&str]) -> WorkspaceState {
        let (mut w, req) = pending();
        w.tree
            .apply_children(req, names.iter().map(|n| table(n)).collect());
        w
    }

    #[test]
    fn refresh_root_then_expand_home_requests_tables_folder() {
        // Arrange
        let (mut w, _) = pending();
        w.tree.refresh_root();

        // Act
        let cmds = w.expand_home();

        // Assert
        assert!(matches!(
            cmds.last(),
            Some(Command::LoadChildren {
                kind: Some(ObjectKind::Table),
                ..
            })
        ));
        assert!(w.tree.loaded_tables("d").is_none());
    }

    fn tables_folder() -> crate::state::tree::NodePath {
        crate::state::tree::NodePath::Folder {
            database: "d".into(),
            schema: Some("public".into()),
            kind: ObjectKind::Table,
        }
    }

    #[test]
    fn refresh_unrelated_node_keeps_tables_collapsed() {
        // Arrange
        let (mut w, _) = pending();
        let obj = |schema: Option<&str>, name: &str, kind| ObjectRef {
            database: "d".into(),
            schema: schema.map(Into::into),
            name: name.into(),
            kind,
        };
        let Some(Command::LoadChildren { req, .. }) = w.tree.refresh_root() else {
            panic!("no roots load");
        };
        w.tree
            .apply_children(req, vec![obj(None, "d", ObjectKind::Database)]);
        let cmds = w.expand_home();
        let reqs: Vec<_> = cmds
            .iter()
            .filter_map(|c| match c {
                Command::LoadChildren { req, .. } => Some(*req),
                _ => None,
            })
            .collect();
        w.tree
            .apply_children(reqs[0], vec![obj(None, "public", ObjectKind::Schema)]);
        w.tree.apply_children(reqs[1], vec![]);
        w.tree.toggle(tables_folder());
        let views = crate::state::tree::NodePath::Folder {
            database: "d".into(),
            schema: Some("public".into()),
            kind: ObjectKind::View,
        };
        w.tree.refresh(views);

        // Act
        let cmds = w.expand_home();

        // Assert
        assert!(cmds.is_empty());
        let row = w.tree.visible().find(|n| n.label == "Tables");
        assert!(row.is_some_and(|n| !n.expanded));
    }

    #[test]
    fn refresh_database_then_expand_home_requests_tables_folder() {
        // Arrange
        let (mut w, _) = pending();
        w.tree
            .refresh(crate::state::tree::NodePath::Database("d".into()));

        // Act
        let cmds = w.expand_home();

        // Assert
        assert!(matches!(
            cmds.last(),
            Some(Command::LoadChildren {
                kind: Some(ObjectKind::Table),
                ..
            })
        ));
    }

    #[test]
    fn workspace_home_while_loading_shows_loading() {
        // Arrange
        let (w, _) = pending();
        let mut done = false;
        let mut h = Harness::new_ui(|ui| {
            if ready(ui, &mut done) {
                show(ui, &w);
            }
        });

        // Act
        h.run();

        // Assert
        h.get_by_label("Loading…");
        assert!(h.query_by_label("No tables").is_none());
    }

    #[test]
    fn workspace_home_failed_load_shows_error_and_retry() {
        // Arrange
        let (mut w, req) = pending();
        w.tree.fail_children(req, "boom".into());
        let mut done = false;
        let mut got = vec![];
        let mut h = Harness::new_ui(|ui| {
            if ready(ui, &mut done) {
                got.extend(show(ui, &w));
            }
        });
        h.run();

        // Act
        h.get_by_label("Retry").click();
        h.run();

        // Assert
        assert!(h.query_by_label_contains("boom").is_some());
        assert!(h.query_by_label("Loading…").is_none());
        assert!(h.query_by_label("No tables").is_none());
        drop(h);
        assert!(got.contains(&Intent::RefreshNode(w.tree.home_tables_node("d"))));
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
