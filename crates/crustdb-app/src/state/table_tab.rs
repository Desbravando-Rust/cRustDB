use crustdb_core::model::{ColumnDetail, ColumnMeta, ForeignKeyInfo, IndexInfo, ObjectRef, Row};
use crustdb_core::query::{Filter, Sort, TableQuery};

use crate::runtime::{Command, RequestId, TabId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableView {
    Data,
    Structure,
    Ddl,
}

/// Catalog data kept for the Structure view.
#[derive(Debug, Default)]
pub struct Structure {
    pub columns: Vec<ColumnDetail>,
    pub indexes: Vec<IndexInfo>,
    pub fks: Vec<ForeignKeyInfo>,
}

#[derive(Debug)]
pub struct TableTab {
    pub id: TabId,
    pub object: ObjectRef,
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Row>,
    pub page: u64,
    pub page_size: u32,
    pub has_more: bool,
    pub estimate: Option<u64>,
    pub sorts: Vec<Sort>,
    pub filters: Vec<Filter>,
    pub loading: Option<RequestId>,
    pub selected_row: Option<usize>,
    pub view: TableView,
    pub structure: Option<Structure>,
    /// Pending `LoadStructure`; the first page is fetched once it arrives.
    pub structure_req: Option<RequestId>,
    pub needs_fetch: bool,
    pub notice: Option<String>,
    /// Pending `LoadDdl` and its result (`view == Ddl`).
    pub ddl_req: Option<RequestId>,
    pub ddl: Option<String>,
}

impl TableTab {
    pub fn new(id: TabId, object: ObjectRef, page_size: u32) -> Self {
        Self {
            id,
            object,
            columns: vec![],
            rows: vec![],
            page: 0,
            page_size,
            has_more: false,
            estimate: None,
            sorts: vec![],
            filters: vec![],
            loading: None,
            selected_row: None,
            view: TableView::Data,
            structure: None,
            structure_req: None,
            needs_fetch: false,
            notice: None,
            ddl_req: None,
            ddl: None,
        }
    }

    pub fn load_structure(&mut self, req: RequestId) -> Command {
        self.structure_req = Some(req);
        Command::LoadStructure { tab: self.id, req }
    }

    pub fn load_ddl(&mut self, req: RequestId) -> Command {
        self.ddl_req = Some(req);
        Command::LoadDdl { tab: self.id, req }
    }

    /// Query columns come from the catalog (PK flags, real types), never from a result header.
    pub fn apply_structure(&mut self, s: Structure) {
        self.columns = s.columns.iter().map(|d| d.meta.clone()).collect();
        self.structure = Some(s);
        self.structure_req = None;
        self.needs_fetch = true;
    }

    pub fn is_loading(&self) -> bool {
        self.loading.is_some() || self.structure_req.is_some() || self.ddl_req.is_some()
    }

    /// Re-sends what failed: the structure if no columns arrived yet, else the current page.
    pub fn retry(&mut self, req: RequestId) -> Command {
        self.notice = None;
        if self.view == TableView::Ddl {
            self.load_ddl(req)
        } else if self.columns.is_empty() {
            self.load_structure(req)
        } else {
            self.fetch(req)
        }
    }

    /// Error, cancellation or crash: nothing is pending anymore.
    pub fn fail(&mut self, text: String) {
        self.loading = None;
        self.structure_req = None;
        self.ddl_req = None;
        self.notice = Some(text);
    }

    pub fn query(&self) -> TableQuery {
        // Stable pagination: without an explicit sort, order by the primary key.
        let sorts = if self.sorts.is_empty() {
            self.columns
                .iter()
                .filter(|c| c.is_primary_key)
                .map(|c| Sort {
                    column: c.name.clone(),
                    descending: false,
                })
                .collect()
        } else {
            self.sorts.clone()
        };
        TableQuery {
            object: self.object.clone(),
            columns: self.columns.clone(),
            filters: self.filters.clone(),
            sorts,
            limit: Some(u64::from(self.page_size) + 1),
            offset: self.page * u64::from(self.page_size),
        }
    }

    /// Marks the tab as loading and builds the command for the current page.
    pub fn fetch(&mut self, req: RequestId) -> Command {
        self.loading = Some(req);
        Command::FetchPage {
            tab: self.id,
            req,
            query: self.query(),
        }
    }

    /// Keeps at most one page; the extra row only signals that a next page exists.
    pub fn apply_page(&mut self, mut rows: Vec<Row>) {
        self.has_more = rows.len() > self.page_size as usize;
        rows.truncate(self.page_size as usize);
        self.rows = rows;
        self.selected_row = None;
        self.loading = None;
        self.notice = None;
    }

    /// Click cycles asc -> desc -> off; shift keeps the other sorted columns.
    pub fn click_header(&mut self, column: &str, shift: bool) {
        let current = self.sorts.iter().find(|s| s.column == column).cloned();
        if !shift {
            self.sorts.clear();
        }
        match current {
            None => self.sorts.push(Sort {
                column: column.into(),
                descending: false,
            }),
            Some(s) if !s.descending => {
                if !shift {
                    self.sorts.push(s);
                }
                if let Some(s) = self.sorts.iter_mut().find(|s| s.column == column) {
                    s.descending = true;
                }
            }
            Some(_) => self.sorts.retain(|s| s.column != column),
        }
        self.page = 0;
    }

    /// Returns whether the page changed.
    pub fn next_page(&mut self) -> bool {
        self.has_more && {
            self.page += 1;
            true
        }
    }

    pub fn prev_page(&mut self) -> bool {
        self.page > 0 && {
            self.page -= 1;
            true
        }
    }

    /// Without a primary key or an explicit sort the server may return rows in any order.
    pub fn unordered(&self) -> bool {
        self.sorts.is_empty() && !self.columns.iter().any(|c| c.is_primary_key)
    }

    pub fn footer_label(&self) -> String {
        if self.rows.is_empty() {
            return if self.is_loading() {
                "Loading…"
            } else {
                "No rows"
            }
            .into();
        }
        let start = self.page * u64::from(self.page_size) + 1;
        let end = start + self.rows.len() as u64 - 1;
        match self.estimate {
            Some(n) if self.filters.is_empty() => format!("Rows {start}–{end} of ~{}", short(n)),
            _ => format!("Rows {start}–{end}"),
        }
    }
}

fn short(n: u64) -> String {
    match n {
        1_000_000.. => format!("{:.1}M", n as f64 / 1e6),
        1_000.. => format!("{:.1}K", n as f64 / 1e3),
        _ => n.to_string(),
    }
}

#[cfg(test)]
mod tests {
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

    fn tab_with(
        page_size: u32,
        page: u64,
        estimate: Option<u64>,
        filters: Vec<Filter>,
    ) -> TableTab {
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
}
