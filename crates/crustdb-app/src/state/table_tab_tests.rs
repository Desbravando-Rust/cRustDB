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
    let mut t = tab(300);
    t.apply_page(rows(301));
    assert!(t.has_more);
    assert_eq!(t.rows.len(), 300);
    t.apply_page(rows(120));
    assert!(!t.has_more);
}

#[test]
fn sort_header_cycle_and_shift_multi() {
    let mut t = tab(300);
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
    let t = tab_with(300, 1, Some(1_200_000), vec![]);
    assert_eq!(t.footer_label(), "Rows 301–600 of ~1.2M");
}

#[test]
fn footer_hides_total_with_filters() {
    let t = tab_with(300, 0, Some(1_200_000), vec![filter_eq("a", "1")]);
    assert_eq!(t.footer_label(), "Rows 1–300");
}

#[test]
fn sort_or_filter_change_resets_to_first_page() {
    let mut t = tab(300);
    t.page = 3;
    t.click_header("a", false);
    assert_eq!(t.page, 0);
}

#[test]
fn query_uses_limit_plus_one_and_page_offset() {
    let mut t = tab(300);
    t.page = 2;
    let q = t.query();
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
    let mut t = tab(300);
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
    let mut t = tab(300);
    t.fetch(RequestId(1));
    t.fail("cancelled".into());
    assert!(t.loading.is_none());
    assert_eq!(t.notice.as_deref(), Some("cancelled"));
}

#[test]
fn retry_reloads_structure_then_page_and_loading_covers_structure() {
    let mut t = tab(300);
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
    let mut t = tab(300);
    assert!(!t.next_page() && !t.prev_page());
    t.has_more = true;
    assert!(t.next_page() && t.prev_page());
}

#[test]
fn unordered_only_without_pk_and_sort() {
    let mut t = tab(300);
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
    let mut t = tab(300);
    assert_eq!(t.bar_error(), None);
    t.filters = vec![filter_eq("a", "x")];
    t.fail("invalid input syntax for type integer".into());
    assert_eq!(t.bar_error(), Some("invalid input syntax for type integer"));
}

#[test]
fn set_view_clears_notice_and_loads_ddl_once() {
    let mut t = tab(300);
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
    let mut t = tab(300);
    t.filters = vec![filter_eq("a", "x")];
    t.view = TableView::Ddl;
    t.fail("ddl failed".into());
    assert_eq!(t.bar_error(), None);
}

#[test]
fn toggles_drafts_and_clear() {
    let mut t = tab(300);
    t.columns = vec![col("a", TypeCategory::Integer)];
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

fn text(s: &str) -> Cell {
    Cell::Text {
        value: s.into(),
        truncated: false,
    }
}

fn editable_tab(kind: ObjectKind, with_pk: bool) -> TableTab {
    let mut t = tab(300);
    t.object.kind = kind;
    let mut id = pk("id");
    id.is_primary_key = with_pk;
    t.columns = vec![id, col("v", TypeCategory::Text)];
    t.rows = vec![vec![text("1"), text("old")].into()];
    t
}

#[test]
fn edit_cell_builds_update_only_for_table_with_pk() {
    let mut t = editable_tab(ObjectKind::Table, true);
    t.begin_edit(0, 1);
    assert_eq!(t.editing, Some((0, 1, "old".into())));
    if let Some((_, _, s)) = t.editing.as_mut() {
        *s = "new".into();
    }
    let Some(Command::UpdateCell { req, update, .. }) = t.commit_edit(RequestId(7)) else {
        panic!("expected UpdateCell");
    };
    assert_eq!(req, RequestId(7));
    assert!(update.sql.starts_with(r#"UPDATE "public"."t" SET "v" = "#));
    assert!(t.editing.is_none());
    // Unchanged value: nothing to send.
    t.begin_edit(0, 1);
    assert!(t.commit_edit(RequestId(8)).is_none());
    assert!(matches!(
        t.set_null(0, 1, RequestId(9)),
        Some(Command::UpdateCell { .. })
    ));

    for mut t in [
        editable_tab(ObjectKind::View, true),
        editable_tab(ObjectKind::Table, false),
    ] {
        t.begin_edit(0, 1);
        assert!(t.editing.is_none());
        assert!(t.set_null(0, 1, RequestId(1)).is_none());
        assert!(t.notice.is_some());
    }
}

#[test]
fn cell_updated_marks_needs_fetch() {
    let mut t = editable_tab(ObjectKind::Table, true);
    let _ = t.set_null(0, 1, RequestId(3));
    t.cell_updated(RequestId(2));
    assert!(!t.needs_fetch, "stale id ignored");
    t.cell_updated(RequestId(3));
    assert!(t.needs_fetch && t.edit_req.is_none());
}

#[test]
fn begin_edit_skips_text_at_memory_cap() {
    let mut t = editable_tab(ObjectKind::Table, true);
    t.rows[0][1] = text(&"a".repeat(crustdb_core::model::CELL_MEMORY_CAP));
    t.begin_edit(0, 1);
    assert!(t.editing.is_none());
}
