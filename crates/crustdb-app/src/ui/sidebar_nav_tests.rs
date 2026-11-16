#![allow(clippy::unwrap_used, clippy::expect_used)]
use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use crustdb_core::model::{ObjectKind, ObjectRef, Product, ServerVersion};

use crate::runtime::{Command, TabId};
use crate::state::tree::NodePath;
use crate::state::workspace::tests::state;
use crate::state::{AppState, Status, TableTab};
use crate::ui::Intent;
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
    s.workspaces[0].tabs.push(TableTab::new(
        TabId(1),
        obj("users", ObjectKind::Table),
        100,
    ));
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
fn explorer_icon_is_decorative() {
    // Arrange
    let s = with_users_leaf();

    // Act
    let h = render(&s);

    // Assert
    assert!(h.query_by_label_contains("users").is_some());
    assert!(
        h.query_by_label(egui_phosphor::regular::TABLE).is_none(),
        "icon glyph is decorative, not an accessible label"
    );
}

#[test]
fn connection_item_has_database_icon_and_left_aligned_name() {
    // Arrange
    let s = with_users_leaf();

    // Act
    let h = render(&s);

    // Assert
    assert!(
        h.query_by_label(egui_phosphor::regular::DATABASE).is_none(),
        "icon glyph is decorative"
    );
    let header = h.get_by_label("CONNECTIONS").rect();
    let name = h.get_by_label("alpha").rect();
    let dot = h.get_by_label("Online").rect();
    assert!(name.left() - header.left() >= 14.0, "room for the icon");
    assert!(name.right() <= dot.left());
    assert!(
        dot.center().x > name.center().x,
        "dot sits right of the name"
    );
}

#[test]
fn clicking_right_of_the_name_switches_workspace() {
    // Arrange
    let mut s = state(&["alpha", "beta"]);
    s.open_workspace(s.config.connections[0].clone());
    let other = s.open_workspace(s.config.connections[1].clone());
    s.active = Some(s.workspaces[0].id);
    s.workspaces[0].status = Status::Online(ServerVersion {
        product: Product::Postgres,
        major: 16,
        minor: 0,
        patch: 0,
        raw: "16.0".into(),
    });
    let mut done = false;
    let mut got = vec![];
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            got.extend(super::show(ui, &s));
        }
    });
    h.run();
    let name = h.get_by_label("beta").rect();

    // Act
    let pos = egui::pos2(name.right() - 4.0, name.center().y);
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
    }
    h.run();

    // Assert
    drop(h);
    assert!(got.contains(&Intent::SwitchWorkspace(other)));
}

fn two_online() -> (AppState, crate::runtime::WorkspaceId) {
    let mut s = state(&["alpha", "beta"]);
    s.open_workspace(s.config.connections[0].clone());
    let other = s.open_workspace(s.config.connections[1].clone());
    s.active = Some(s.workspaces[0].id);
    s.workspaces[0].status = Status::Online(ServerVersion {
        product: Product::Postgres,
        major: 16,
        minor: 0,
        patch: 0,
        raw: "16.0".into(),
    });
    (s, other)
}

#[test]
fn connection_row_is_one_tab_stop_and_enter_switches() {
    // Arrange
    let (s, other) = two_online();
    let mut done = false;
    let mut got = vec![];
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            got.extend(super::show(ui, &s));
        }
    });
    h.run();
    h.get_by_label("alpha").focus();
    h.run();

    // Act
    h.key_press(egui::Key::Tab);
    h.run();
    let on_beta = h.get_by_label("beta").accesskit_node().is_focused();
    h.key_press(egui::Key::Enter);
    h.run();

    // Assert
    assert!(on_beta, "one Tab from alpha lands on beta, no phantom stop");
    drop(h);
    assert!(got.contains(&Intent::SwitchWorkspace(other)));
}

#[test]
fn long_connection_name_truncates_and_keeps_dot_and_chip() {
    // Arrange
    let (mut s, _) = two_online();
    s.workspaces[0].config.name = "a-very-long-connection-name-".repeat(6);
    s.config.connections[0].label = Some("prod".into());
    let mut done = false;
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            super::show(ui, &s);
        }
    });

    // Act
    h.run();

    // Assert
    let name = h.get_by_label_contains("a-very-long").rect();
    let chip = h.get_by_label("prod").rect();
    let dot = h.get_by_label("Online").rect();
    assert!(name.right() <= chip.left(), "name stops before the chip");
    assert!(chip.right() <= dot.left() && dot.width() > 0.0);
}
