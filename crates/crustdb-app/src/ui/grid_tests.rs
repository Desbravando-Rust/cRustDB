#![allow(clippy::unwrap_used, clippy::expect_used)]
use crustdb_core::model::{ObjectKind, ObjectRef, TypeCategory};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use super::*;
use crate::runtime::TabId;
use crate::ui::data_toolbar::{COMMIT_LABEL, FILTER_HINT};
use egui_phosphor::regular as icon;

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
    let mut h = Harness::new_ui(|ui| {
        crate::ui::theme::install_theme(ui.ctx());
        out.extend(show(ui, t));
    });
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
    let mut h = Harness::new_ui(|ui| {
        crate::ui::theme::install_theme(ui.ctx());
        out.extend(show(ui, &mut t));
    });
    h.run();

    // Act
    h.get_by_label(COMMIT_LABEL).click();
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
    let mut h = Harness::new_ui(|ui| {
        crate::ui::theme::install_theme(ui.ctx());
        out.extend(show(ui, &mut t));
    });
    h.run();

    // Act
    h.get_by_label(COMMIT_LABEL).click();
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
fn commit_changes_disabled_without_pending() {
    // Arrange
    let mut t = tab();
    let mut h = Harness::new_ui(|ui| {
        crate::ui::theme::install_theme(ui.ctx());
        drop(show(ui, &mut t));
    });

    // Act
    h.run();

    // Assert
    assert!(disabled(&h, COMMIT_LABEL));
}

#[test]
fn pending_cell_shows_staged_value() {
    // Arrange
    let mut t = staged();
    let mut h = Harness::new_ui(|ui| {
        crate::ui::theme::install_theme(ui.ctx());
        drop(show(ui, &mut t));
    });

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
    let mut h = Harness::new_ui(|ui| {
        crate::ui::theme::install_theme(ui.ctx());
        drop(show(ui, &mut t));
    });

    // Act
    h.run();

    // Assert
    assert!(h.query_by_label("Retry").is_none());
    assert!(h.query_by_label(icon::ARROW_CLOCKWISE).is_some());
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
        let mut h = Harness::new_ui(|ui| {
            crate::ui::theme::install_theme(ui.ctx());
            drop(show(ui, &mut t));
        });

        // Act
        h.run();

        // Assert
        assert!(disabled(&h, icon::PLUS));
        assert!(disabled(&h, "Delete rows"));
    }
}

#[test]
fn add_row_button_emits_intent() {
    // Arrange
    let mut t = tab();
    let mut out = vec![];
    let mut h = Harness::new_ui(|ui| {
        crate::ui::theme::install_theme(ui.ctx());
        out.extend(show(ui, &mut t));
    });
    h.run();

    // Act
    h.get_by_label(icon::PLUS).click();
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
    let mut h = Harness::new_ui(|ui| {
        crate::ui::theme::install_theme(ui.ctx());
        out.extend(show(ui, &mut t));
    });
    h.run();
    assert!(disabled(&h, "Delete rows"));
    drop(h);
    t.selected_row = Some(0);
    let mut h = Harness::new_ui(|ui| {
        crate::ui::theme::install_theme(ui.ctx());
        out.extend(show(ui, &mut t));
    });
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
    let mut h = Harness::new_ui(|ui| {
        crate::ui::theme::install_theme(ui.ctx());
        drop(show(ui, &mut t));
    });

    // Act
    h.run();

    // Assert
    assert!(h.query_by_label("Discard").is_some());
    assert!(disabled(&h, icon::PLUS));
}

#[test]
fn empty_insert_row_shows_save_and_new_row_label() {
    // Arrange
    let mut t = tab();
    t.editing = None;
    t.add_row();
    let mut out = vec![];
    let mut h = Harness::new_ui(|ui| {
        crate::ui::theme::install_theme(ui.ctx());
        out.extend(show(ui, &mut t));
    });
    h.run();

    // Act
    h.get_by_label(COMMIT_LABEL).click();
    h.run();
    let new_row = h.query_by_label("New row").is_some();
    let zero = h.query_by_label("0 pending").is_some();
    drop(h);

    // Assert
    assert!(
        out.iter()
            .any(|i| matches!(i, Intent::SavePending(TabId(1))))
    );
    assert!(new_row && !zero);
}

const NO_PK_NOTICE: &str = "Read-only: table has no primary key";

#[test]
fn table_without_pk_shows_read_only_notice() {
    // Arrange
    let mut t = tab();
    t.columns[0].is_primary_key = false;
    let mut h = Harness::new_ui(|ui| {
        crate::ui::theme::install_theme(ui.ctx());
        drop(show(ui, &mut t));
    });

    // Act
    h.run();

    // Assert
    assert!(h.query_by_label_contains(NO_PK_NOTICE).is_some());
}

#[test]
fn table_with_pk_has_no_read_only_notice() {
    // Arrange
    let mut with_pk = tab();
    let mut loading = tab();
    loading.columns.clear();
    loading.rows.clear();
    loading.editing = None;
    for t in [&mut with_pk, &mut loading] {
        let mut h = Harness::new_ui(|ui| {
            crate::ui::theme::install_theme(ui.ctx());
            drop(show(ui, t));
        });

        // Act
        h.run();

        // Assert
        assert!(h.query_by_label_contains(NO_PK_NOTICE).is_none());
    }
}

#[test]
fn grid_header_shows_muted_column_type() {
    // Arrange
    let mut t = tab();
    t.editing = None;
    t.columns[0].type_name = "INT".into();
    let mut h = Harness::new_ui(|ui| {
        crate::ui::theme::install_theme(ui.ctx());
        drop(show(ui, &mut t));
    });

    // Act
    h.run();

    // Assert
    assert!(h.query_by_label("INT").is_some());
    assert!(h.query_by_label("id").is_some());
}

#[test]
fn grid_shows_row_numbers() {
    // Arrange
    let mut t = tab();
    t.editing = None;
    t.rows = vec![
        vec![Cell::text_capped("a"), Cell::text_capped("b")].into(),
        vec![Cell::text_capped("c"), Cell::text_capped("d")].into(),
    ];
    let mut h = Harness::new_ui(|ui| {
        crate::ui::theme::install_theme(ui.ctx());
        drop(show(ui, &mut t));
    });

    // Act
    h.run();

    // Assert
    assert!(h.query_by_label("#").is_some());
    assert!(h.query_by_label("1").is_some());
    assert!(h.query_by_label("2").is_some());
}

#[test]
fn footer_shows_changes_pending_count() {
    // Arrange
    let mut t = staged();
    let mut h = Harness::new_ui(|ui| {
        crate::ui::theme::install_theme(ui.ctx());
        drop(show(ui, &mut t));
    });

    // Act
    h.run();

    // Assert
    assert!(h.query_by_label("1 change pending").is_some());
}

fn luma(c: Color32) -> f32 {
    f32::from(c.r()) * 0.299 + f32::from(c.g()) * 0.587 + f32::from(c.b()) * 0.114
}

#[test]
fn integer_color_is_legible_on_selected_row() {
    // Arrange
    let selection = crate::ui::theme::ACCENT;

    // Act & Assert
    assert_ne!(INT_COLOR, selection);
    assert!(luma(INT_COLOR) - luma(selection) > 80.0);
}

#[test]
fn toolbar_filter_rows_opens_filter_bar() {
    // Arrange
    let mut t = tab();
    t.editing = None;
    let mut h = Harness::builder()
        // TODO(#186): drop once the filter TextInput has an accessible name.
        .with_accessibility_check(false)
        .build_ui(|ui| {
            crate::ui::theme::install_theme(ui.ctx());
            drop(show(ui, &mut t));
        });
    h.run();

    // Act
    h.get_by_label(FILTER_HINT).click();
    h.run();
    drop(h);

    // Assert
    assert!(t.filter_open);
    assert!(!t.drafts.is_empty());
}

#[test]
fn toolbar_refresh_emits_retry() {
    // Arrange
    let mut t = tab();
    t.editing = None;
    let mut out = vec![];
    let mut h = Harness::new_ui(|ui| {
        crate::ui::theme::install_theme(ui.ctx());
        out.extend(show(ui, &mut t));
    });
    h.run();

    // Act
    h.get_by_label(icon::ARROW_CLOCKWISE).click();
    h.run();
    drop(h);

    // Assert
    assert!(out.iter().any(|i| matches!(i, Intent::Retry(TabId(1)))));
}

#[test]
fn footer_has_no_save_or_refresh_button() {
    // Arrange
    let mut t = staged();
    let mut h = Harness::new_ui(|ui| {
        crate::ui::theme::install_theme(ui.ctx());
        drop(show(ui, &mut t));
    });

    // Act
    h.run();

    // Assert
    assert!(h.query_by_label("Save").is_none());
    assert!(h.query_by_label("Refresh").is_none());
    assert!(h.query_by_label("Add row").is_none());
}
