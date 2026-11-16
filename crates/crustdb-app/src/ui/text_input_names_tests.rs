#![allow(clippy::unwrap_used, clippy::expect_used)]
use crustdb_core::driver::ResultHeader;
use crustdb_core::model::DriverKind;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use crate::runtime::{TabId, WorkspaceId};
use crate::state::QueryTab;
use crate::state::command_pane::CommandPane;
use crate::state::keyspace::Keyspace;
use crate::state::workspace::tests::state;
use crate::storage::config::Label;
use crate::ui::home::tests::ready;

#[test]
fn command_pane_input_is_named() {
    // Arrange
    let mut pane = CommandPane::default();
    let mut h = Harness::new_ui(|ui| {
        super::command_pane::show(ui, &mut pane);
    });

    // Act
    h.run();

    // Assert
    h.get_by_role_and_label(Role::TextInput, "Command");
}

#[test]
fn history_search_is_named() {
    // Arrange
    let mut panel = super::history_panel::HistoryPanel::default();
    let mut h = Harness::new_ui(|ui| {
        super::history_panel::show(ui, &mut panel);
    });

    // Act
    h.run();

    // Assert
    h.get_by_role_and_label(Role::TextInput, "Search history");
}

#[test]
fn home_search_is_named() {
    // Arrange
    let s = state(&["alpha"]);
    let mut done = false;
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            super::home::show(ui, &s);
        }
    });

    // Act
    h.run();

    // Assert
    h.get_by_role_and_label(Role::TextInput, "Search connections");
}

#[test]
fn key_pattern_is_named() {
    // Arrange
    let ks = Keyspace::new(WorkspaceId(1), "0".into());
    let mut h = Harness::new_ui(|ui| {
        super::keyspace::sidebar(ui, &ks);
    });

    // Act
    h.run();

    // Assert
    h.get_by_role_and_label(Role::TextInput, "Key pattern");
}

#[test]
fn query_toolbar_database_and_result_filter_are_named() {
    // Arrange
    let mut tab = QueryTab::new(TabId(1), DriverKind::Postgres, None);
    tab.pane.header = Some(ResultHeader { columns: vec![] });
    let mut h = Harness::new_ui(|ui| {
        super::results::toolbar(ui, &mut tab);
        super::results::show(ui, &mut tab);
    });

    // Act
    h.run();

    // Assert
    h.get_by_role_and_label(Role::TextInput, "Database name");
    h.get_by_role_and_label(Role::TextInput, "Filter rows");
}

fn labels() -> Vec<Label> {
    ["dev", "prod"]
        .iter()
        .map(|n| Label {
            name: (*n).into(),
            color: [1, 2, 3],
        })
        .collect()
}

#[test]
fn label_editor_rows_have_unique_names() {
    // Arrange
    let labels = labels();
    let mut h = Harness::new_ui(|ui| {
        super::labels::editor(ui, &labels, false);
    });
    h.run();

    // Act
    h.get_by_label("Labels...").click();
    h.run();

    // Assert
    h.get_by_role_and_label(Role::TextInput, "Label name dev");
    h.get_by_role_and_label(Role::TextInput, "Label name prod");
    h.get_by_role_and_label(Role::TextInput, "New label name");
}

#[test]
fn label_filter_search_is_named() {
    // Arrange
    let labels = labels();
    let mut h = Harness::new_ui(|ui| {
        super::labels::filter_combo(ui, &labels);
    });
    h.run();

    // Act
    h.get_by_role_and_label(Role::ComboBox, "Filter").click();
    h.run();

    // Assert
    h.get_by_role_and_label(Role::TextInput, "Search labels");
}
