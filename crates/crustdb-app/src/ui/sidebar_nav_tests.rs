#![allow(clippy::unwrap_used, clippy::expect_used)]
use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use crustdb_core::model::{ObjectKind, ObjectRef, Product, ServerVersion};

use crate::runtime::{Command, TabId};
use crate::state::tree::NodePath;
use crate::state::workspace::tests::state;
use crate::state::{AppState, Status, Tab, TableTab};
use crate::ui::home::tests::ready;

fn obj(name: &str, kind: ObjectKind) -> ObjectRef {
    ObjectRef {
        database: "app".into(),
        schema: Some("public".into()),
        name: name.into(),
        kind,
    }
}

/// Online Postgres workspace whose tree shows `app > public > Tables > users`.
fn with_users_leaf() -> AppState {
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
    let t = &mut s.workspaces[0].tree;
    let Some(Command::LoadChildren { req, .. }) = t.load_roots() else {
        panic!("roots");
    };
    t.apply_children(
        req,
        vec![ObjectRef {
            database: "app".into(),
            schema: None,
            name: "app".into(),
            kind: ObjectKind::Database,
        }],
    );
    let schema_req = |t: &mut crate::state::tree::ObjectTree, p| {
        let Some(Command::LoadChildren { req, .. }) = t.expand(p) else {
            panic!("expand");
        };
        req
    };
    let r = schema_req(t, NodePath::Database("app".into()));
    t.apply_children(r, vec![obj("public", ObjectKind::Schema)]);
    t.expand(NodePath::Schema("app".into(), "public".into()));
    let folder = NodePath::Folder {
        database: "app".into(),
        schema: Some("public".into()),
        kind: ObjectKind::Table,
    };
    let r = schema_req(t, folder);
    t.apply_children(r, vec![obj("users", ObjectKind::Table)]);
    s
}

fn open_users(s: &mut AppState) {
    s.workspaces[0].tabs.push(Tab::Table(TableTab::new(
        TabId(1),
        obj("users", ObjectKind::Table),
        100,
    )));
}

fn render(s: &AppState) -> Harness<'_, ()> {
    let mut done = false;
    let mut h = Harness::new_ui(move |ui| {
        if ready(ui, &mut done) {
            super::show(ui, s);
        }
    });
    h.run();
    h
}

#[test]
fn explorer_open_table_is_selected() {
    // Arrange
    let mut s = with_users_leaf();
    open_users(&mut s);

    // Act
    let h = render(&s);

    // Assert
    assert_eq!(
        h.get_by_label_contains("users").accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
}

#[test]
fn explorer_closed_table_is_not_selected() {
    // Arrange
    let s = with_users_leaf();

    // Act
    let h = render(&s);

    // Assert
    assert_eq!(
        h.get_by_label_contains("users").accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::False)
    );
}

#[test]
fn explorer_table_row_has_table_icon() {
    // Arrange
    let s = with_users_leaf();

    // Act
    let h = render(&s);

    // Assert
    assert!(
        h.query_by_label(egui_phosphor::regular::TABLE).is_some(),
        "table icon next to the leaf"
    );
}

#[test]
fn connection_item_has_database_icon_and_left_aligned_name() {
    // Arrange
    let s = with_users_leaf();

    // Act
    let h = render(&s);

    // Assert
    let icon = h.get_by_label(egui_phosphor::regular::DATABASE).rect();
    let name = h.get_by_label("alpha").rect();
    let dot = h.get_by_label("Online").rect();
    assert!(icon.right() <= name.left());
    assert!(name.right() <= dot.left());
    assert!(
        dot.right() > name.right() + 20.0,
        "dot sits at the right edge"
    );
}
