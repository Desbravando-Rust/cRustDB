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
    let got = click(&s, "File", "New connection…");

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
    let refresh = click(&s, "View", "Refresh data");
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
    assert!(is_disabled(&off, "View", "Refresh data"));
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

fn with_table_tab(s: &mut AppState) {
    let object = crustdb_core::model::ObjectRef {
        database: "d".into(),
        schema: None,
        name: "t".into(),
        kind: crustdb_core::model::ObjectKind::Table,
    };
    s.workspaces[0].open_table(TabId(1), object, crate::runtime::RequestId(1));
}

#[test]
fn page_items_need_a_table_tab() {
    // Arrange
    let query_tab = online(true);
    let mut table_tab = online(false);
    with_table_tab(&mut table_tab);

    // Act & Assert
    assert!(is_disabled(&query_tab, "View", "Next page"));
    assert!(is_disabled(&query_tab, "View", "Previous page"));
    assert!(!is_disabled(&table_tab, "View", "Next page"));
    assert_eq!(
        click(&table_tab, "View", "Previous page"),
        vec![MenuCmd::Action(Action::PrevPage)]
    );
}

#[test]
fn quit_asks_the_window_to_close() {
    // Arrange
    let s = offline();
    let mut done = false;
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            show(ui, &s);
        }
    });
    h.run();
    h.get_by_label("File").click();
    h.run();

    // Act
    h.get_by_label_contains("Quit").click();
    h.step();

    // Assert
    let closed = h
        .output()
        .viewport_output
        .values()
        .any(|v| v.commands.contains(&egui::ViewportCommand::Close));
    assert!(closed);
}

#[test]
fn help_items_open_https_urls() {
    // Arrange
    let s = offline();

    for (item, url) in [
        ("Releases", crate::news::RELEASES_URL),
        ("README", README_URL),
    ] {
        let mut done = false;
        let mut h = Harness::new_ui(|ui| {
            if ready(ui, &mut done) {
                show(ui, &s);
            }
        });
        h.run();
        h.get_by_label("Help").click();
        h.run();

        // Act
        h.get_by_label_contains(item).click();
        h.step();

        // Assert
        let opened = h.output().platform_output.commands.iter().any(
            |c| matches!(c, egui::OutputCommand::OpenUrl(u) if u.url == url && u.url.starts_with("https://")),
        );
        assert!(opened, "{item}");
    }
}

#[test]
fn new_query_is_disabled_with_reason_without_sql_editor() {
    // Arrange
    let mut s = online(false);
    s.workspaces[0].config.driver = crustdb_core::model::DriverKind::Redis;
    let mut done = false;
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            show(ui, &s);
        }
    });
    h.run();
    h.get_by_label("File").click();
    h.run();

    // Act
    h.get_by_label_contains("New query").hover();
    h.run();

    // Assert
    assert!(
        h.get_by_label_contains("New query")
            .accesskit_node()
            .is_disabled()
    );
    h.get_by_label("Not supported for Redis");
}

#[test]
fn menu_names_do_not_collide_with_sidebar() {
    // Arrange
    let s = online(true);
    let mut done = false;
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            show(ui, &s);
            crate::ui::sidebar::show(ui, &s);
        }
    });
    h.run();
    h.get_by_label("File").click();
    h.run();

    // Act
    let mut names: Vec<_> = h
        .query_all_by_role(egui::accesskit::Role::Button)
        .filter_map(|n| n.accesskit_node().label())
        .collect();
    names.sort();

    // Assert
    let total = names.len();
    names.dedup();
    assert_eq!(names.len(), total, "duplicate button names: {names:?}");
}
