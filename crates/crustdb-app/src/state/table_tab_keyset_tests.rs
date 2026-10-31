#![allow(clippy::unwrap_used, clippy::expect_used)]
use crustdb_core::model::{Cell, ObjectKind, TypeCategory};

use super::*;

fn col(name: &str, cat: TypeCategory, pk: bool, nullable: Option<bool>) -> ColumnMeta {
    ColumnMeta {
        name: name.into(),
        type_name: "t".into(),
        category: cat,
        nullable,
        is_primary_key: pk,
    }
}

fn tab() -> TableTab {
    let object = ObjectRef {
        database: "d".into(),
        schema: None,
        name: "t".into(),
        kind: ObjectKind::Table,
    };
    let mut t = TableTab::new(TabId(1), object, 2);
    t.columns = vec![
        col("id", TypeCategory::Integer, true, Some(false)),
        col("n", TypeCategory::Text, false, None),
    ];
    t
}

fn row(id: &str) -> Row {
    let text = |s: &str| Cell::Text {
        value: s.into(),
        truncated: false,
    };
    vec![text(id), text("x")].into()
}

/// Server page of `size + 1` rows (more to come) starting at `first`.
fn page(first: u32, size: u32) -> Vec<Row> {
    (first..first + size + 1)
        .map(|i| row(&i.to_string()))
        .collect()
}

#[test]
fn next_prev_next_reuse_anchors() {
    // Arrange
    let mut t = tab();
    t.apply_page(page(1, 2));

    // Act & Assert
    assert!(t.next_page());
    assert_eq!(t.query().after, Some(vec!["2".to_string()]));
    t.apply_page(page(3, 2));
    assert!(t.next_page());
    assert_eq!(t.query().after, Some(vec!["4".to_string()]));
    assert!(t.prev_page());
    assert_eq!(t.query().after, Some(vec!["2".to_string()]));
    assert!(t.prev_page());
    assert_eq!(t.query().after, None);
}

#[test]
fn reload_keeps_anchor() {
    // Arrange
    let mut t = tab();
    t.apply_page(page(1, 2));
    t.next_page();
    t.apply_page(page(3, 2));

    // Act
    t.apply_page(page(3, 2));

    // Assert
    assert_eq!(t.query().after, Some(vec!["2".to_string()]));
    t.next_page();
    assert_eq!(t.query().after, Some(vec!["4".to_string()]));
}

#[test]
fn query_adds_pk_tiebreak_after_sort() {
    // Arrange
    let mut t = tab();
    t.click_header("n", false);

    // Act
    let sorts = t.query().sorts;

    // Assert
    assert_eq!(sorts.len(), 2);
    assert_eq!(sorts[1].column, "id");
    assert!(!sorts[1].descending);
    assert_eq!(t.sorts.len(), 1);
}

#[test]
fn sort_and_filter_changes_reset_anchors() {
    // Arrange
    let mut t = tab();
    t.apply_page(page(1, 2));
    t.next_page();

    // Act
    t.click_header("id", false);

    // Assert
    assert_eq!(t.page, 0);
    t.apply_page(page(1, 2));
    t.next_page();
    t.clear_filters(RequestId(1));
    assert_eq!(t.query().after, None);
    assert_eq!(t.page, 0);
}

#[test]
fn nullable_sort_column_falls_back_to_offset() {
    // Arrange
    let mut t = tab();
    t.click_header("n", false);
    t.apply_page(page(1, 2));

    // Act
    t.next_page();

    // Assert
    let q = t.query();
    assert_eq!(q.after, None);
    assert_eq!(q.offset, 2);
}
