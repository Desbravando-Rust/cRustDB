#![allow(clippy::unwrap_used, clippy::expect_used)]
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use crustdb_core::model::{Product, ServerVersion};

use super::*;
use crate::runtime::TabId;
use crate::state::Status;
use crate::state::workspace::tests::state;
use crate::ui::home::tests::ready;

fn offline() -> AppState {
    state(&["alpha"])
}

fn online(tab: bool) -> AppState {
    let mut s = offline();
    let ws = s.open_workspace(s.config.connections[0].clone());
    s.workspaces[0].status = Status::Online(ServerVersion {
        product: Product::Postgres,
        major: 16,
        minor: 0,
        patch: 0,
        raw: "16.0".into(),
    });
    if tab {
        s.workspaces[0].active_tab = Some(TabId(1));
    }
    s.active = Some(ws);
    s
}

/// Opens `menu`, clicks `item` and returns what the bar emitted.
fn click(s: &AppState, menu: &str, item: &str) -> Vec<MenuCmd> {
    let mut done = false;
    let mut got = vec![];
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            got.extend(show(ui, s));
        }
    });
    h.run();
    h.get_by_label(menu).click();
    h.run();
    h.get_by_label_contains(item).click();
    h.run();
    drop(h);
    got
}

fn is_disabled(s: &AppState, menu: &str, item: &str) -> bool {
    let mut done = false;
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            show(ui, s);
        }
    });
    h.run();
    h.get_by_label(menu).click();
    h.run();
    h.get_by_label_contains(item).accesskit_node().is_disabled()
}

#[test]
fn new_connection_works_offline() {
    // Arrange
    let s = offline();

    // Act
    let got = click(&s, "File", "New connection");

    // Assert
    assert_eq!(got, vec![MenuCmd::Action(Action::NewConnection)]);
}

#[test]
fn disconnect_targets_the_active_workspace() {
    // Arrange
    let s = online(false);
    let ws = s.workspaces[0].id;

    // Act
    let got = click(&s, "File", "Disconnect");

    // Assert
    assert_eq!(got, vec![MenuCmd::Intent(Intent::Disconnect(ws))]);
}

#[test]
fn view_and_tools_items_fire_their_actions() {
    // Arrange
    let s = online(true);

    // Act
    let refresh = click(&s, "View", "Refresh");
    let history = click(&s, "Tools", "Query history");
    let find = click(&s, "Edit", "Find / filter");

    // Assert
    assert_eq!(refresh, vec![MenuCmd::Action(Action::Refresh)]);
    assert_eq!(history, vec![MenuCmd::Action(Action::History)]);
    assert_eq!(find, vec![MenuCmd::Action(Action::Filter)]);
}

#[test]
fn items_are_disabled_when_not_applicable() {
    // Arrange
    let off = offline();
    let no_tab = online(false);

    // Act & Assert
    assert!(is_disabled(&off, "File", "New query"));
    assert!(is_disabled(&off, "File", "Disconnect"));
    assert!(is_disabled(&off, "View", "Refresh"));
    assert!(is_disabled(&no_tab, "File", "Close tab"));
    assert!(!is_disabled(&no_tab, "File", "New query"));
    assert!(!is_disabled(&online(true), "File", "Close tab"));
}

#[test]
fn disabled_item_does_not_fire() {
    // Arrange
    let s = offline();

    // Act
    let got = click(&s, "File", "New query");

    // Assert
    assert!(got.is_empty());
}

#[test]
fn items_show_their_shortcut() {
    // Arrange
    let s = online(true);
    let mut done = false;
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            show(ui, &s);
        }
    });
    h.run();

    // Act
    h.get_by_label("File").click();
    h.run();

    // Assert
    let want = h
        .ctx
        .format_shortcut(&keymap::shortcut(Action::NewQuery).unwrap());
    assert!(h.query_by_label_contains(&want).is_some());
}
