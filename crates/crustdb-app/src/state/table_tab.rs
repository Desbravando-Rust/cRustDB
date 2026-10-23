use crustdb_core::model::{
    ColumnDetail, ColumnMeta, DriverKind, ForeignKeyInfo, IndexInfo, ObjectRef, Row,
};
use crustdb_core::query::{Filter, Sort, TableQuery};

use super::filter_draft::FilterDraft;
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
    pub driver: DriverKind,
    pub filter_open: bool,
    pub drafts: Vec<FilterDraft>,
    pub filter_error: Option<String>,
    pub detail_open: bool,
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
            driver: DriverKind::Postgres,
            filter_open: false,
            drafts: vec![],
            filter_error: None,
            detail_open: false,
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

    pub fn apply_filters(&mut self, req: RequestId) -> Option<Command> {
        let filters: Result<Vec<_>, _> = self
            .drafts
            .iter()
            .map(|d| d.to_filter(&self.columns, self.driver))
            .collect();
        match filters {
            Ok(f) => {
                self.filters = f;
                self.filter_error = None;
                self.page = 0;
                Some(self.fetch(req))
            }
            Err(e) => {
                self.filter_error = Some(e);
                None
            }
        }
    }

    /// What the filter bar shows: a draft error, or the server's rejection of the applied filters.
    pub fn bar_error(&self) -> Option<&str> {
        self.filter_error.as_deref().or(self
            .notice
            .as_deref()
            .filter(|_| self.view == TableView::Data && !self.filters.is_empty()))
    }

    /// Switching views drops the other view's error; DDL is loaded only the first time.
    pub fn set_view(&mut self, view: TableView, req: RequestId) -> Option<Command> {
        if self.view == view {
            return None;
        }
        self.view = view;
        self.notice = None;
        (view == TableView::Ddl && self.ddl.is_none() && self.ddl_req.is_none())
            .then(|| self.load_ddl(req))
    }

    pub fn toggle_filter_bar(&mut self) {
        self.filter_open = !self.filter_open;
        if self.filter_open && self.drafts.is_empty() {
            self.add_draft();
        }
    }

    pub fn toggle_detail(&mut self) {
        self.detail_open = !self.detail_open;
    }

    pub fn add_draft(&mut self) {
        self.drafts.push(FilterDraft::new(&self.columns));
    }

    pub fn remove_draft(&mut self, i: usize) {
        if i < self.drafts.len() {
            self.drafts.remove(i);
        }
    }

    /// Empties the drafts and reapplies, back to page 0.
    pub fn clear_filters(&mut self, req: RequestId) -> Command {
        self.drafts.clear();
        self.filters.clear();
        self.filter_error = None;
        self.page = 0;
        self.fetch(req)
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

    pub fn click_header(&mut self, column: &str, shift: bool) {
        toggle_sort(&mut self.sorts, column, shift);
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

/// Click cycles asc -> desc -> off; shift keeps the other sorted columns.
pub(crate) fn toggle_sort(sorts: &mut Vec<Sort>, column: &str, shift: bool) {
    let current = sorts.iter().find(|s| s.column == column).cloned();
    if !shift {
        sorts.clear();
    }
    match current {
        None => sorts.push(Sort {
            column: column.into(),
            descending: false,
        }),
        Some(s) if !s.descending => {
            if !shift {
                sorts.push(s);
            }
            if let Some(s) = sorts.iter_mut().find(|s| s.column == column) {
                s.descending = true;
            }
        }
        Some(_) => sorts.retain(|s| s.column != column),
    }
}

#[cfg(test)]
#[path = "table_tab_tests.rs"]
mod tests;
