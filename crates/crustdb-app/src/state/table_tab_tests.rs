#![allow(clippy::unwrap_used, clippy::expect_used)]
use crustdb_core::model::{Cell, ObjectKind, TypeCategory};
use crustdb_core::query::Operator;

use super::*;

fn tab(page_size: u32) -> TableTab {
    let object = ObjectRef {
        database: "d".into(),
        schema: Some("public".into()),
        name: "t".into(),
        kind: ObjectKind::Table,
    };
    TableTab::new(TabId(1), object, page_size)
}

fn tab_with(page_size: u32, page: u64, estimate: Option<u64>, filters: Vec<Filter>) -> TableTab {
    let mut t = tab(page_size);
    t.page = page;
    t.estimate = estimate;
    t.filters = filters;
    t.rows = rows(page_size as usize);
    t
}

fn rows(n: usize) -> Vec<Row> {
    (0..n).map(|_| vec![Cell::Null].into()).collect()
}

fn asc(c: &str) -> Sort {
    Sort {
        column: c.into(),
        descending: false,
    }
}

fn desc(c: &str) -> Sort {
    Sort {
        column: c.into(),
        descending: true,
    }
}

fn filter_eq(c: &str, v: &str) -> Filter {
    Filter {
        column: c.into(),
        op: Operator::Eq,
        values: vec![v.into()],
    }
}

#[test]
fn pagination_next_enabled_only_when_limit_plus_one() {
    // Arrange
    let mut t = tab(300);

    // Act & Assert
    t.apply_page(rows(301));
    assert!(t.has_more);
    assert_eq!(t.rows.len(), 300);
    t.apply_page(rows(120));
    assert!(!t.has_more);
}

#[test]
fn sort_header_cycle_and_shift_multi() {
    // Arrange
    let mut t = tab(300);

    // Act & Assert
    t.click_header("a", false);
    assert_eq!(t.sorts, [asc("a")]);
    t.click_header("a", false);
    assert_eq!(t.sorts, [desc("a")]);
    t.click_header("b", true);
    assert_eq!(t.sorts, [desc("a"), asc("b")]);
    t.click_header("a", false);
    assert!(t.sorts.is_empty());
}

#[test]
fn footer_shows_estimate_without_filters() {
    // Arrange
    let t = tab_with(300, 1, Some(1_200_000), vec![]);

    // Act & Assert
    assert_eq!(t.footer_label(), "Rows 301–600 of ~1.2M");
}

#[test]
fn footer_hides_total_with_filters() {
    // Arrange
    let t = tab_with(300, 0, Some(1_200_000), vec![filter_eq("a", "1")]);

    // Act & Assert
    assert_eq!(t.footer_label(), "Rows 1–300");
}

#[test]
fn sort_or_filter_change_resets_to_first_page() {
    // Arrange
    let mut t = tab(300);
    t.page = 3;

    // Act
    t.click_header("a", false);

    // Assert
    assert_eq!(t.page, 0);
}

#[test]
fn query_uses_limit_plus_one_and_page_offset() {
    // Arrange
    let mut t = tab(300);
    t.page = 2;

    // Act
    let q = t.query();

    // Assert
    assert_eq!((q.limit, q.offset), (Some(301), 600));
}

fn pk(name: &str) -> ColumnMeta {
    ColumnMeta {
        name: name.into(),
        type_name: "int4".into(),
        category: TypeCategory::Integer,
        nullable: Some(false),
        is_primary_key: true,
    }
}

#[test]
fn structure_sets_catalog_columns_and_query_orders_by_pk() {
    // Arrange
    let mut t = tab(300);

    // Act & Assert
    t.apply_structure(Structure {
        columns: vec![ColumnDetail {
            meta: pk("id"),
            default: None,
            comment: None,
        }],
        ..Default::default()
    });
    assert!(t.needs_fetch && t.structure_req.is_none());
    let q = t.query();
    assert_eq!(q.sorts, [asc("id")]);
    assert!(q.columns[0].is_primary_key);
    assert!(t.sorts.is_empty());
}

#[test]
fn fail_clears_loading_and_keeps_message() {
    // Arrange
    let mut t = tab(300);
    t.fetch(RequestId(1));

    // Act
    t.fail("cancelled".into());

    // Assert
    assert!(t.loading.is_none());
    assert_eq!(t.notice.as_deref(), Some("cancelled"));
}

#[test]
fn retry_reloads_structure_then_page_and_loading_covers_structure() {
    // Arrange
    let mut t = tab(300);

    // Act & Assert
    t.load_structure(RequestId(1));
    assert!(t.is_loading());
    assert_eq!(t.footer_label(), "Loading…");
    t.fail("boom".into());
    assert_eq!(t.footer_label(), "No rows");
    assert!(matches!(
        t.retry(RequestId(2)),
        Command::LoadStructure { .. }
    ));
    assert!(t.notice.is_none());
    t.columns.push(pk("id"));
    assert!(matches!(t.retry(RequestId(3)), Command::FetchPage { .. }));
}

#[test]
fn page_moves_report_change() {
    // Arrange
    let mut t = tab(300);

    // Act & Assert
    assert!(!t.next_page() && !t.prev_page());
    t.has_more = true;
    assert!(t.next_page() && t.prev_page());
}

#[test]
fn unordered_only_without_pk_and_sort() {
    // Arrange
    let mut t = tab(300);

    // Act & Assert
    assert!(t.unordered());
    t.click_header("a", false);
    assert!(!t.unordered());
    t.sorts.clear();
    t.columns.push(ColumnMeta {
        name: "id".into(),
        type_name: "int".into(),
        category: TypeCategory::Integer,
        nullable: Some(false),
        is_primary_key: true,
    });
    assert!(!t.unordered());
}

fn col(name: &str, category: TypeCategory) -> ColumnMeta {
    ColumnMeta {
        name: name.into(),
        type_name: "t".into(),
        category,
        nullable: None,
        is_primary_key: false,
    }
}

fn draft(column: &str, op: Operator, input: &str) -> FilterDraft {
    let (column, input) = (column.into(), input.into());
    FilterDraft { column, op, input }
}

#[test]
fn apply_sends_fetch_with_and_combined_filters() {
    // Arrange
    let mut t = tab(300);
    t.columns = vec![
        col("a", TypeCategory::Integer),
        col("b", TypeCategory::Text),
    ];
    t.page = 3;
    t.drafts = vec![
        draft("a", Operator::Gt, "1"),
        draft("b", Operator::Like, "x%"),
    ];

    // Act & Assert
    let Some(Command::FetchPage { query, .. }) = t.apply_filters(RequestId(9)) else {
        panic!("expected FetchPage");
    };
    assert_eq!(query.filters.len(), 2);
    assert_eq!(query.offset, 0);
    assert_eq!(t.loading, Some(RequestId(9)));
    t.drafts[0].input.clear();
    assert!(t.apply_filters(RequestId(10)).is_none());
    assert_eq!(t.bar_error(), Some("value required"));
}

#[test]
fn server_filter_error_shown_in_bar() {
    // Arrange
    let mut t = tab(300);

    // Act & Assert
    assert_eq!(t.bar_error(), None);
    t.filters = vec![filter_eq("a", "x")];
    t.fail("invalid input syntax for type integer".into());
    assert_eq!(t.bar_error(), Some("invalid input syntax for type integer"));
}

#[test]
fn set_view_clears_notice_and_loads_ddl_once() {
    // Arrange
    let mut t = tab(300);

    // Act & Assert
    t.fail("data error".into());
    assert!(matches!(
        t.set_view(TableView::Ddl, RequestId(1)),
        Some(Command::LoadDdl { .. })
    ));
    assert!(t.notice.is_none());
    assert!(t.set_view(TableView::Data, RequestId(2)).is_none());
    assert!(t.set_view(TableView::Ddl, RequestId(3)).is_none());
    t.ddl_req = None;
    t.ddl = Some("x".into());
    t.set_view(TableView::Data, RequestId(4));
    assert!(t.set_view(TableView::Ddl, RequestId(5)).is_none());
}

#[test]
fn bar_error_only_from_data_view() {
    // Arrange
    let mut t = tab(300);
    t.filters = vec![filter_eq("a", "x")];
    t.view = TableView::Ddl;

    // Act
    t.fail("ddl failed".into());

    // Assert
    assert_eq!(t.bar_error(), None);
}

#[test]
fn toggles_drafts_and_clear() {
    // Arrange
    let mut t = tab(300);
    t.columns = vec![col("a", TypeCategory::Integer)];

    // Act & Assert
    t.toggle_filter_bar();
    assert!(t.filter_open && t.drafts.len() == 1);
    t.add_draft();
    t.remove_draft(0);
    t.remove_draft(9);
    assert_eq!(t.drafts.len(), 1);
    t.toggle_detail();
    assert!(t.detail_open);
    t.filters = vec![filter_eq("a", "1")];
    t.page = 2;
    let Command::FetchPage { query, .. } = t.clear_filters(RequestId(1)) else {
        panic!()
    };
    assert!(query.filters.is_empty() && query.offset == 0 && t.drafts.is_empty());
}
