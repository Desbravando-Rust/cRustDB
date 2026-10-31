#![allow(clippy::unwrap_used, clippy::expect_used)]
use crustdb_core::model::{ObjectKind, ObjectRef, TypeCategory};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use super::*;
use crate::runtime::TabId;

fn tab() -> TableTab {
    let object = ObjectRef {
        database: "d".into(),
        schema: None,
        name: "t".into(),
        kind: ObjectKind::Table,
    };
    let mut t = TableTab::new(TabId(1), object, 300);
    t.columns = ["id", "v"]
        .iter()
        .map(|n| ColumnMeta {
            name: (*n).into(),
            type_name: "text".into(),
            category: TypeCategory::Text,
            nullable: None,
            is_primary_key: *n == "id",
        })
        .collect();
    t.rows = vec![vec![Cell::text_capped("1"), Cell::text_capped("old")].into()];
    t.editing = Some((0, 1, "old".into()));
    t
}

fn run_keys(t: &mut TableTab, text: &str, key: egui::Key) -> Vec<Intent> {
    let mut out = vec![];
    let mut h = Harness::new_ui(|ui| out.extend(show(ui, t)));
    h.run();
    h.event(egui::Event::Text(text.into()));
    h.run();
    h.key_press(key);
    h.run();
    drop(h);
    out
}

#[test]
fn enter_in_cell_editor_commits_typed_text() {
    // Arrange
    let mut t = tab();

    // Act
    let out = run_keys(&mut t, "x", egui::Key::Enter);

    // Assert
    assert!(
        out.iter()
            .any(|i| matches!(i, Intent::CommitEdit(TabId(1))))
    );
    assert!(t.editing.unwrap().2.contains('x'));
}

#[test]
fn escape_in_cell_editor_cancels() {
    // Arrange
    let mut t = tab();

    // Act
    let out = run_keys(&mut t, "x", egui::Key::Escape);

    // Assert
    assert!(out.is_empty());
    assert!(t.editing.is_none());
}

fn staged() -> TableTab {
    let mut t = tab();
    t.editing = None;
    t.pending = vec![(1, Some("new".into()))];
    t.pending_row = Some(0);
    t
}

#[test]
fn save_and_discard_buttons_emit_intents_when_pending() {
    // Arrange
    let mut t = staged();
    let mut out = vec![];
    let mut h = Harness::new_ui(|ui| out.extend(show(ui, &mut t)));
    h.run();

    // Act
    h.get_by_label("Save").click();
    h.run();
    h.get_by_label("Discard").click();
    h.run();
    drop(h);

    // Assert
    assert!(
        out.iter()
            .any(|i| matches!(i, Intent::SavePending(TabId(1))))
    );
    assert!(out.iter().any(|i| matches!(i, Intent::DiscardPending(_))));
}

#[test]
fn save_commits_open_cell_editor_first() {
    // Arrange
    let mut t = staged();
    t.editing = Some((0, 1, "typed".into()));
    let mut out = vec![];
    let mut h = Harness::new_ui(|ui| out.extend(show(ui, &mut t)));
    h.run();

    // Act
    h.get_by_label("Save").click();
    h.run();
    drop(h);

    // Assert
    assert!(
        out.iter()
            .any(|i| matches!(i, Intent::SavePending(TabId(1))))
    );
    assert!(t.editing.is_none());
    assert_eq!(t.pending, vec![(1, Some("typed".into()))]);
}

#[test]
fn no_save_button_without_pending() {
    // Arrange
    let mut t = tab();
    let mut h = Harness::new_ui(|ui| drop(show(ui, &mut t)));

    // Act
    h.run();

    // Assert
    assert!(h.query_by_label("Save").is_none());
}

#[test]
fn pending_cell_shows_staged_value() {
    // Arrange
    let mut t = staged();
    let mut h = Harness::new_ui(|ui| drop(show(ui, &mut t)));

    // Act
    h.run();

    // Assert
    assert!(h.query_by_label("new").is_some());
    assert!(h.query_by_label("old").is_none());
}

#[test]
fn data_view_notice_has_no_duplicate_retry() {
    // Arrange
    let mut t = tab();
    t.notice = Some("boom".into());
    let mut h = Harness::new_ui(|ui| drop(show(ui, &mut t)));

    // Act
    h.run();

    // Assert
    assert!(h.query_by_label("Retry").is_none());
    assert!(h.query_by_label("Refresh").is_some());
}

fn disabled(h: &Harness<'_>, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().is_disabled()
}

#[test]
fn add_and_delete_disabled_for_view_or_no_pk() {
    // Arrange
    let mut view = tab();
    view.object.kind = ObjectKind::View;
    let mut no_pk = tab();
    no_pk.columns[0].is_primary_key = false;
    for mut t in [view, no_pk] {
        t.selected_row = Some(0);
        let mut h = Harness::new_ui(|ui| drop(show(ui, &mut t)));

        // Act
        h.run();

        // Assert
        assert!(disabled(&h, "Add row"));
        assert!(disabled(&h, "Delete rows"));
    }
}

#[test]
fn add_row_button_emits_intent() {
    // Arrange
    let mut t = tab();
    let mut out = vec![];
    let mut h = Harness::new_ui(|ui| out.extend(show(ui, &mut t)));
    h.run();

    // Act
    h.get_by_label("Add row").click();
    h.run();
    drop(h);

    // Assert
    assert!(out.iter().any(|i| matches!(i, Intent::AddRow(TabId(1)))));
}

#[test]
fn delete_button_emits_intent_with_selection() {
    // Arrange
    let mut t = tab();
    let mut out = vec![];
    let mut h = Harness::new_ui(|ui| out.extend(show(ui, &mut t)));
    h.run();
    assert!(disabled(&h, "Delete rows"));
    drop(h);
    t.selected_row = Some(0);
    let mut h = Harness::new_ui(|ui| out.extend(show(ui, &mut t)));
    h.run();

    // Act
    h.get_by_label("Delete rows").click();
    h.run();
    drop(h);

    // Assert
    assert!(
        out.iter()
            .any(|i| matches!(i, Intent::DeleteRows(TabId(1))))
    );
}

#[test]
fn discard_shows_for_empty_insert_row() {
    // Arrange
    let mut t = tab();
    t.editing = None;
    t.add_row();
    let mut h = Harness::new_ui(|ui| drop(show(ui, &mut t)));

    // Act
    h.run();

    // Assert
    assert!(h.query_by_label("Discard").is_some());
    assert!(disabled(&h, "Add row"));
}
