#![allow(clippy::unwrap_used, clippy::expect_used)]
use crustdb_core::driver::Param;
use crustdb_core::model::{ColumnMeta, ObjectRef, TypeCategory};

use super::*;
use crate::runtime::TabId;

const NOTICE: &str = "Save or discard pending changes";

fn col(name: &str, pk: bool) -> ColumnMeta {
    ColumnMeta {
        name: name.into(),
        type_name: "t".into(),
        category: TypeCategory::Text,
        nullable: None,
        is_primary_key: pk,
    }
}

fn text(s: &str) -> Cell {
    Cell::Text {
        value: s.into(),
        truncated: false,
    }
}

/// Table `t(id pk, v, w)` with rows `(1, old, x)` and `(2, b, y)`.
fn editable_tab(kind: ObjectKind, with_pk: bool) -> TableTab {
    let object = ObjectRef {
        database: "d".into(),
        schema: Some("public".into()),
        name: "t".into(),
        kind,
    };
    let mut t = TableTab::new(TabId(1), object, 300);
    t.columns = vec![col("id", with_pk), col("v", false), col("w", false)];
    t.rows = vec![
        vec![text("1"), text("old"), text("x")].into(),
        vec![text("2"), text("b"), text("y")].into(),
    ];
    t
}

fn type_and_commit(t: &mut TableTab, row: usize, col: usize, s: &str) {
    t.begin_edit(row, col);
    if let Some((_, _, buf)) = t.editing.as_mut() {
        *buf = s.into();
    }
    t.commit_edit();
}

#[test]
fn commit_edit_stages_without_command() {
    // Arrange
    let mut t = editable_tab(ObjectKind::Table, true);

    // Act
    type_and_commit(&mut t, 0, 1, "a");
    t.set_null(0, 2);
    let cmd = t.save_pending(RequestId(7));

    // Assert
    assert!(t.editing.is_none());
    assert_eq!(t.pending_row, Some(0));
    assert_eq!(t.pending, vec![(1, Some("a".into())), (2, None)]);
    let Some(Command::UpdateCell { req, update, .. }) = cmd else {
        panic!("expected one UpdateCell");
    };
    assert_eq!(req, RequestId(7));
    assert_eq!(t.edit_req, Some(RequestId(7)));
    assert!(update.sql.starts_with(r#"UPDATE "public"."t" SET "v" = "#));
    assert!(update.sql.contains(r#", "w" = "#));
    assert_eq!(
        update.params,
        vec![
            Param(Some("a".into())),
            Param(None),
            Param(Some("1".into()))
        ]
    );
}

#[test]
fn re_edit_replaces_and_original_value_unstages() {
    // Arrange
    let mut t = editable_tab(ObjectKind::Table, true);

    // Act & Assert
    type_and_commit(&mut t, 0, 1, "old");
    assert!(t.pending.is_empty() && t.pending_row.is_none());
    type_and_commit(&mut t, 0, 1, "a");
    t.begin_edit(0, 1);
    assert_eq!(t.editing, Some((0, 1, "a".into())), "shows staged value");
    t.editing = None;
    type_and_commit(&mut t, 0, 1, "b");
    assert_eq!(t.pending, vec![(1, Some("b".into()))]);
    type_and_commit(&mut t, 0, 1, "old");
    assert!(t.pending.is_empty() && t.pending_row.is_none());
    assert!(t.save_pending(RequestId(1)).is_none());
}

#[test]
fn discard_pending_clears_changes() {
    // Arrange
    let mut t = editable_tab(ObjectKind::Table, true);
    type_and_commit(&mut t, 0, 1, "a");

    // Act
    t.discard_pending();

    // Assert
    assert!(t.pending.is_empty() && t.pending_row.is_none());
    assert!(t.save_pending(RequestId(1)).is_none());
}

#[test]
fn rejects_view_or_table_without_pk() {
    for mut t in [
        editable_tab(ObjectKind::View, true),
        editable_tab(ObjectKind::Table, false),
    ] {
        // Act
        t.begin_edit(0, 1);
        t.set_null(0, 1);

        // Assert
        assert!(t.editing.is_none());
        assert!(t.pending.is_empty());
        assert!(t.notice.is_some());
    }
}

#[test]
fn editing_another_row_with_pending_is_blocked() {
    // Arrange
    let mut t = editable_tab(ObjectKind::Table, true);
    type_and_commit(&mut t, 0, 1, "a");

    // Act
    t.begin_edit(1, 1);
    t.set_null(1, 2);

    // Assert
    assert!(t.editing.is_none());
    assert_eq!(t.pending, vec![(1, Some("a".into()))]);
    assert_eq!(t.pending_row, Some(0));
    assert_eq!(t.notice.as_deref(), Some(NOTICE));
}

#[test]
fn navigation_with_pending_is_blocked() {
    // Arrange
    let mut t = editable_tab(ObjectKind::Table, true);
    t.has_more = true;
    t.page = 1;
    type_and_commit(&mut t, 0, 1, "a");

    // Act & Assert
    assert!(!t.next_page());
    assert!(!t.prev_page());
    assert!(!t.click_header("v", false));
    assert!(t.apply_filters(RequestId(1)).is_none());
    assert!(t.clear_filters(RequestId(2)).is_none());
    assert!(t.retry(RequestId(3)).is_none());
    assert!(t.pending_blocks());
    assert_eq!((t.page, t.loading), (1, None));
    assert!(t.sorts.is_empty());
    assert_eq!(t.notice.as_deref(), Some(NOTICE));
}

#[test]
fn cell_updated_success_clears_pending_and_reloads() {
    // Arrange
    let mut t = editable_tab(ObjectKind::Table, true);
    t.set_null(0, 1);
    let _ = t.save_pending(RequestId(3));

    // Act & Assert
    t.cell_updated(RequestId(2), None);
    assert!(!t.needs_fetch, "stale id ignored");
    t.cell_updated(RequestId(3), None);
    assert!(t.needs_fetch && t.edit_req.is_none());
    assert!(t.pending.is_empty() && t.pending_row.is_none());
}

#[test]
fn update_error_keeps_pending_and_fetch_in_flight() {
    // Arrange
    let mut t = editable_tab(ObjectKind::Table, true);
    t.set_null(0, 1);
    let _ = t.save_pending(RequestId(3));
    t.fetch(RequestId(4));

    // Act
    t.cell_updated(RequestId(3), Some("boom".into()));

    // Assert
    assert_eq!(t.loading, Some(RequestId(4)));
    assert_eq!(t.notice.as_deref(), Some("boom"));
    assert!(!t.needs_fetch && t.edit_req.is_none());
    assert_eq!(t.pending, vec![(1, None)]);
}

#[test]
fn begin_edit_skips_text_at_memory_cap() {
    // Arrange
    let mut t = editable_tab(ObjectKind::Table, true);
    t.rows[0][1] = text(&"a".repeat(crustdb_core::model::CELL_MEMORY_CAP));

    // Act
    t.begin_edit(0, 1);

    // Assert
    assert!(t.editing.is_none());
}

#[test]
fn discard_pending_clears_only_the_pending_notice() {
    // Arrange
    let mut t = editable_tab(ObjectKind::Table, true);
    type_and_commit(&mut t, 0, 1, "a");
    t.notice = Some(NOTICE.into());

    // Act
    t.discard_pending();

    // Assert
    assert!(t.notice.is_none());

    // Arrange
    t.notice = Some("boom".into());

    // Act
    t.discard_pending();

    // Assert
    assert_eq!(t.notice.as_deref(), Some("boom"));
}
