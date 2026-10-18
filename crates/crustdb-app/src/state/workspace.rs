use std::collections::HashMap;
use std::ops::Range;
use std::time::Instant;

use crustdb_core::model::{ConnectionId, ObjectRef, ServerVersion};
use secrecy::SecretString;

use super::autocomplete::ColumnCache;
use super::dialogs::{Dialog, DialogStack};
use super::query_tab::{QueryTab, char_to_byte};
use super::table_tab::{Structure, TableTab, TableView};
use super::tree::{ObjectTree, OpenAction, open_action};
use crate::runtime::{Command, Event, RequestId, TabId, WorkspaceId};
use crate::storage::config::{ConnectionConfig, LoadedConfig};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Connecting,
    Online(ServerVersion),
    Failed(String),
}

#[derive(Debug)]
pub struct WorkspaceState {
    pub id: WorkspaceId,
    pub config: ConnectionConfig,
    pub status: Status,
    pub tree: ObjectTree,
    pub columns: ColumnCache,
    pub tabs: Vec<Tab>,
    /// Query editor tabs; ids come from the same counter as `tabs`.
    pub queries: Vec<QueryTab>,
    pub active_tab: Option<TabId>,
    /// The password this session connected with, for the safety prompt (never read from the keyring).
    pub session_password: Option<SecretString>,
}

#[derive(Debug)]
pub enum Tab {
    Table(TableTab),
}

impl Tab {
    pub fn id(&self) -> TabId {
        match self {
            Tab::Table(t) => t.id,
        }
    }
}

impl WorkspaceState {
    /// Opens a table tab; the first page is fetched after the catalog structure arrives.
    pub fn open_table(&mut self, id: TabId, object: ObjectRef, req: RequestId) -> [Command; 2] {
        let mut tab = TableTab::new(id, object.clone(), self.config.page_size);
        tab.driver = self.config.driver;
        // ponytail: DDL objects reuse TableTab with view Ddl until a dedicated tab type exists.
        let load = if open_action(object.kind) == OpenAction::Ddl {
            tab.view = TableView::Ddl;
            tab.load_ddl(req)
        } else {
            tab.load_structure(req)
        };
        self.tabs.push(Tab::Table(tab));
        self.active_tab = Some(id);
        [
            Command::OpenTable {
                ws: self.id,
                tab: id,
                object,
            },
            load,
        ]
    }

    pub fn active_table_mut(&mut self) -> Option<&mut TableTab> {
        let id = self.active_tab?;
        self.tabs
            .iter_mut()
            .map(|Tab::Table(t)| t)
            .find(|t| t.id == id)
    }

    pub fn active_query_mut(&mut self) -> Option<&mut QueryTab> {
        let id = self.active_tab?;
        self.queries.iter_mut().find(|q| q.id == id)
    }

    /// The connection form stays up until the workspace is online.
    pub fn form_open(&self) -> bool {
        !matches!(self.status, Status::Online(_))
    }
}

pub struct AppState {
    pub config: LoadedConfig,
    pub workspaces: Vec<WorkspaceState>,
    pub active: Option<WorkspaceId>,
    pub dialogs: DialogStack,
    next_ws: u64,
    next_tab: u64,
    evaluating: HashMap<RequestId, (WorkspaceId, TabId)>,
    next_exec: u64,
    outbox: Vec<Command>,
}

impl AppState {
    pub fn new(config: LoadedConfig) -> Self {
        Self {
            config,
            workspaces: vec![],
            active: None,
            dialogs: DialogStack::default(),
            // 0 is the runtime's sentinel.
            next_ws: 1,
            next_tab: 1,
            evaluating: HashMap::new(),
            // High range so these ids cannot collide with the tabs' own counters.
            next_exec: 1 << 62,
            outbox: vec![],
        }
    }

    /// Id for a workspace that never appears in the UI (connection test).
    pub fn reserve_ws(&mut self) -> WorkspaceId {
        let id = WorkspaceId(self.next_ws);
        self.next_ws += 1;
        id
    }

    /// One counter for every workspace: the runtime routes by `TabId` alone.
    pub fn next_tab_id(&mut self) -> TabId {
        let id = TabId(self.next_tab);
        self.next_tab += 1;
        id
    }

    /// Entry point for `Intent::OpenObject`: `OpenTable` + `LoadStructure`.
    pub fn open_table(
        &mut self,
        ws: WorkspaceId,
        object: ObjectRef,
        req: RequestId,
    ) -> Vec<Command> {
        let id = self.next_tab_id();
        self.workspace_mut(ws)
            .map(|w| {
                let open = w
                    .tabs
                    .iter()
                    .map(|Tab::Table(t)| t)
                    .find(|t| t.object == object);
                match open {
                    Some(t) => {
                        w.active_tab = Some(t.id);
                        vec![]
                    }
                    None => w.open_table(id, object, req).into(),
                }
            })
            .unwrap_or_default()
    }

    /// Entry point for `Intent::NewQuery`: a blank editor tab.
    pub fn open_query(&mut self, ws: WorkspaceId, database: Option<String>) -> Option<Command> {
        let id = self.next_tab_id();
        let w = self.workspace_mut(ws)?;
        w.queries
            .push(QueryTab::new(id, w.config.driver, database.clone()));
        w.active_tab = Some(id);
        Some(Command::OpenEditor {
            ws,
            tab: id,
            database,
        })
    }

    /// Entry point for `Intent::RunQuery`: always `Evaluate`, never `Execute` (the dialog stack decides).
    /// `run` is in chars (selection, or an empty range at the cursor); converted against the real text.
    /// Refuses while an Evaluate for the tab is pending, so a key-repeat cannot execute twice.
    pub fn run_query(
        &mut self,
        tab: TabId,
        run: Range<usize>,
        all: bool,
        req: RequestId,
    ) -> Option<Command> {
        if self.evaluating.values().any(|(_, t)| *t == tab) {
            return None;
        }
        let (ws, q) = self.workspaces.iter_mut().find_map(|w| {
            let id = w.id;
            Some((id, w.queries.iter_mut().find(|q| q.id == tab)?))
        })?;
        let bytes = char_to_byte(&q.text, run.start)..char_to_byte(&q.text, run.end);
        let selection = (!bytes.is_empty()).then(|| bytes.clone());
        let (starts, statements): (Vec<_>, Vec<_>) = q
            .target_spans(bytes.end, selection, all)
            .into_iter()
            .unzip();
        if statements.is_empty() || !q.can_run() {
            return None;
        }
        q.starts = starts;
        self.register_evaluate(req, ws, tab);
        Some(Command::Evaluate {
            tab,
            req,
            statements,
        })
    }

    pub fn query_mut(&mut self, id: TabId) -> Option<&mut QueryTab> {
        self.workspaces
            .iter_mut()
            .flat_map(|w| w.queries.iter_mut())
            .find(|q| q.id == id)
    }

    /// Called for every command that is actually sent: an `Execute` starts the tab's run.
    pub fn note_sent(&mut self, cmd: &Command) {
        if let Command::Execute { tab, req, .. } = cmd
            && let Some(q) = self.query_mut(*tab)
        {
            q.begin_run(*req);
        }
        if let Command::CloseTab { tab, .. } = cmd {
            for w in &mut self.workspaces {
                w.queries.retain(|q| q.id != *tab);
                w.active_tab = w.active_tab.filter(|t| t != tab);
            }
        }
    }

    pub fn table_mut(&mut self, id: TabId) -> Option<&mut TableTab> {
        self.workspaces
            .iter_mut()
            .flat_map(|w| w.tabs.iter_mut())
            .map(|Tab::Table(t)| t)
            .find(|t| t.id == id)
    }

    /// Commands for tabs whose catalog structure just arrived: their first page.
    pub fn take_fetches(&mut self, mut next_req: impl FnMut() -> RequestId) -> Vec<Command> {
        self.workspaces
            .iter_mut()
            .flat_map(|w| w.tabs.iter_mut())
            .map(|Tab::Table(t)| t)
            .filter_map(|t| std::mem::take(&mut t.needs_fetch).then_some(t))
            .map(|t| t.fetch(next_req()))
            .collect()
    }

    pub fn open_workspace(&mut self, config: ConnectionConfig) -> WorkspaceId {
        let id = WorkspaceId(self.next_ws);
        self.next_ws += 1;
        self.workspaces.push(WorkspaceState {
            id,
            tree: ObjectTree::new(id, config.driver),
            columns: ColumnCache::default(),
            config,
            status: Status::Connecting,
            tabs: vec![],
            queries: vec![],
            active_tab: None,
            session_password: None,
        });
        self.active = Some(id);
        id
    }

    pub fn active_online(&self) -> Option<&WorkspaceState> {
        let id = self.active?;
        self.workspaces
            .iter()
            .find(|w| w.id == id && matches!(w.status, Status::Online(_)))
    }

    /// Called by whoever sends `Command::Evaluate`, so the answer finds its workspace and tab.
    pub fn register_evaluate(&mut self, req: RequestId, ws: WorkspaceId, tab: TabId) {
        self.evaluating.insert(req, (ws, tab));
    }

    /// Commands produced by applied events (e.g. an approved-by-default `Execute`).
    pub fn take_commands(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.outbox)
    }

    pub fn workspace_mut(&mut self, id: WorkspaceId) -> Option<&mut WorkspaceState> {
        self.workspaces.iter_mut().find(|w| w.id == id)
    }

    pub fn apply(&mut self, e: Event) {
        match e {
            Event::Connected { ws, server } => {
                if let Some(w) = self.workspace_mut(ws) {
                    w.status = Status::Online(server);
                    w.tree.reset();
                }
            }
            Event::ConnectFailed { ws, error } => {
                if let Some(w) = self.workspace_mut(ws) {
                    w.status = Status::Failed(error);
                }
            }
            Event::Columns { req, columns } => {
                for w in &mut self.workspaces {
                    w.columns.apply(req, columns.clone());
                }
            }
            Event::Children { req, items } => {
                for w in &mut self.workspaces {
                    w.tree.apply_children(req, items.clone());
                }
            }
            Event::Page {
                req,
                header,
                rows,
                has_more,
                estimate,
            } => {
                // A response whose request is no longer `loading` is stale.
                let tab = self
                    .workspaces
                    .iter_mut()
                    .flat_map(|w| w.tabs.iter_mut())
                    .map(|Tab::Table(t)| t)
                    .find(|t| t.loading == Some(req));
                if let Some(t) = tab {
                    // Header columns are display-only; the query schema is the catalog's.
                    let _ = header;
                    t.apply_page(rows);
                    t.has_more |= has_more;
                    t.estimate = estimate;
                }
            }
            Event::Structure {
                req,
                columns,
                indexes,
                fks,
            } => {
                let tab = self
                    .workspaces
                    .iter_mut()
                    .flat_map(|w| w.tabs.iter_mut())
                    .map(|Tab::Table(t)| t)
                    .find(|t| t.structure_req == Some(req));
                if let Some(t) = tab {
                    t.apply_structure(Structure {
                        columns,
                        indexes,
                        fks,
                    });
                }
            }
            Event::Notice { tab, text } => {
                if let Some(t) = self.table_mut(tab) {
                    t.fail(text);
                } else if let Some(q) = self.query_mut(tab) {
                    q.apply(&Event::Notice { tab, text });
                }
            }
            Event::ResultStarted { req, .. }
            | Event::Rows { req, .. }
            | Event::StatementDone { req, .. }
            | Event::ExportDone { req, .. }
            | Event::ExecError { req, .. } => {
                if let Some(q) = self
                    .workspaces
                    .iter_mut()
                    .flat_map(|w| w.queries.iter_mut())
                    .find(|q| q.pane.req == Some(req))
                {
                    q.apply(&e);
                }
            }
            Event::TxChanged { tab, .. } => {
                if let Some(q) = self.query_mut(tab) {
                    q.apply(&e);
                }
            }
            Event::Ddl { req, text } => {
                let tab = self
                    .workspaces
                    .iter_mut()
                    .flat_map(|w| w.tabs.iter_mut())
                    .map(|Tab::Table(t)| t)
                    .find(|t| t.ddl_req == Some(req));
                if let Some(t) = tab {
                    t.ddl = Some(text);
                    t.ddl_req = None;
                    t.notice = None;
                }
            }
            Event::ActorCrashed { tab } => {
                if let Some(t) = self.table_mut(tab) {
                    t.fail("connection actor crashed".into());
                } else if let Some(q) = self.query_mut(tab) {
                    q.apply(&e);
                }
            }
            Event::Evaluated { req, evaluation } => {
                let Some((ws, tab)) = self.evaluating.remove(&req) else {
                    return;
                };
                let exec = RequestId(self.next_exec);
                self.next_exec += 1;
                let cmd = self
                    .dialogs
                    .on_evaluated(ws, tab, req, exec, evaluation, Instant::now());
                self.outbox.extend(cmd);
            }
            Event::HostKeyPrompt {
                ws,
                host,
                fingerprint,
            } => self.dialogs.push(Dialog::HostKey {
                ws,
                host,
                fingerprint,
            }),
            _ => {}
        }
    }

    pub fn request_delete(&mut self, id: ConnectionId) {
        if self.config.read_only {
            return;
        }
        if let Some(c) = self.config.connections.iter().find(|c| c.id == id) {
            self.dialogs.push(Dialog::DeleteConnection {
                id,
                name: c.name.clone(),
            });
        }
    }

    /// Pops the confirmation dialog; the connection is removed only if `save` of the remaining
    /// list succeeds. Returns the deleted id.
    pub fn confirm_delete<E>(
        &mut self,
        save: impl FnOnce(&[ConnectionConfig]) -> Result<(), E>,
    ) -> Result<Option<ConnectionId>, E> {
        let Some(Dialog::DeleteConnection { id, .. }) = self.dialogs.pop() else {
            return Ok(None);
        };
        let rest: Vec<_> = self
            .config
            .connections
            .iter()
            .filter(|c| c.id != id)
            .cloned()
            .collect();
        save(&rest)?;
        self.config.connections = rest;
        Ok(Some(id))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crustdb_core::driver::ResultHeader;
    use crustdb_core::driver::{TlsConfig, TlsMode};
    use crustdb_core::model::{
        ColumnDetail, ColumnMeta, DriverKind, ObjectKind, ObjectRef, Product, TypeCategory,
    };

    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(1);

    pub(crate) fn conn(name: &str) -> ConnectionConfig {
        ConnectionConfig {
            id: ConnectionId(uuid::Uuid::from_u128(
                NEXT.fetch_add(1, Ordering::Relaxed).into(),
            )),
            name: name.into(),
            driver: DriverKind::Postgres,
            host: "localhost".into(),
            port: 5432,
            user: "u".into(),
            database: None,
            save_password: false,
            tls: TlsConfig {
                mode: TlsMode::VerifyFull,
                ca_path: None,
            },
            ssh: None,
            page_size: 300,
            editor_row_limit: 1000,
            safety_threshold: 1000,
        }
    }

    pub(crate) fn state(names: &[&str]) -> AppState {
        AppState::new(LoadedConfig {
            connections: names.iter().map(|n| conn(n)).collect(),
            read_only: false,
            warning: None,
        })
    }

    #[test]
    fn apply_connected_marks_workspace_online() {
        let mut s = state(&["a"]);
        let ws = s.open_workspace(s.config.connections[0].clone());
        assert_eq!(ws, WorkspaceId(1));
        let server = ServerVersion {
            product: Product::Postgres,
            major: 16,
            minor: 0,
            patch: 0,
            raw: "16.0".into(),
        };
        s.apply(Event::Connected {
            ws,
            server: server.clone(),
        });
        assert_eq!(s.workspaces[0].status, Status::Online(server));
        assert!(!s.workspaces[0].form_open());
    }

    fn obj() -> ObjectRef {
        ObjectRef {
            database: "d".into(),
            schema: None,
            name: "t".into(),
            kind: ObjectKind::Table,
        }
    }

    fn detail(name: &str, pk: bool) -> ColumnDetail {
        ColumnDetail {
            meta: ColumnMeta {
                name: name.into(),
                type_name: "int4".into(),
                category: TypeCategory::Integer,
                nullable: Some(false),
                is_primary_key: pk,
            },
            default: None,
            comment: None,
        }
    }

    fn page(req: u64) -> Event {
        Event::Page {
            req: RequestId(req),
            header: ResultHeader {
                columns: vec![detail("x", false).meta],
            },
            rows: vec![],
            has_more: true,
            estimate: Some(5),
        }
    }

    #[test]
    fn open_table_loads_structure_then_fetches_with_catalog_columns() {
        let mut s = state(&["a"]);
        let ws = s.open_workspace(s.config.connections[0].clone());
        let cmds = s.open_table(ws, obj(), RequestId(7));
        assert!(matches!(cmds[0], Command::OpenTable { tab: TabId(1), .. }));
        assert!(matches!(
            cmds[1],
            Command::LoadStructure {
                req: RequestId(7),
                ..
            }
        ));
        assert!(s.take_fetches(|| RequestId(8)).is_empty());
        s.apply(Event::Structure {
            req: RequestId(7),
            columns: vec![detail("id", true)],
            indexes: vec![],
            fks: vec![],
        });
        let f = s.take_fetches(|| RequestId(8));
        let [Command::FetchPage { req, query, .. }] = &f[..] else {
            panic!("expected one FetchPage");
        };
        assert_eq!(*req, RequestId(8));
        assert!(query.columns[0].is_primary_key);
        assert!(s.take_fetches(|| RequestId(9)).is_empty());
    }

    fn func() -> ObjectRef {
        ObjectRef {
            kind: ObjectKind::Function,
            ..obj()
        }
    }

    #[test]
    fn open_object_function_emits_open_and_ddl() {
        let mut s = state(&["a"]);
        let ws = s.open_workspace(s.config.connections[0].clone());
        let cmds = s.open_table(ws, func(), RequestId(1));
        assert!(matches!(
            cmds[..],
            [
                Command::OpenTable { tab: TabId(1), .. },
                Command::LoadDdl {
                    tab: TabId(1),
                    req: RequestId(1)
                }
            ]
        ));
        let Some(Tab::Table(t)) = s.workspaces[0].tabs.first() else {
            panic!("tab");
        };
        assert_eq!(t.view, TableView::Ddl);
        assert!(s.take_fetches(|| RequestId(2)).is_empty());
    }

    #[test]
    fn tab_ids_are_unique_across_workspaces_and_kinds() {
        let mut s = state(&["a", "b"]);
        let a = s.open_workspace(s.config.connections[0].clone());
        let b = s.open_workspace(s.config.connections[1].clone());
        let mut ids = vec![];
        for (ws, o) in [(a, obj()), (b, func()), (a, func()), (b, obj())] {
            for c in s.open_table(ws, o, RequestId(1)) {
                if let Command::OpenTable { tab, .. } = c {
                    ids.push(tab.0);
                }
            }
        }
        assert_eq!(ids, [1, 2, 3, 4]);
    }

    #[test]
    fn page_ignores_stale_req_and_keeps_catalog_columns() {
        let mut s = state(&["a"]);
        let ws = s.open_workspace(s.config.connections[0].clone());
        s.open_table(ws, obj(), RequestId(7));
        s.apply(Event::Structure {
            req: RequestId(7),
            columns: vec![detail("id", true)],
            indexes: vec![],
            fks: vec![],
        });
        s.take_fetches(|| RequestId(8));
        s.apply(page(6));
        let t = s.table_mut(TabId(1)).unwrap();
        assert!(t.loading.is_some() && t.estimate.is_none());
        s.apply(page(8));
        let t = s.table_mut(TabId(1)).unwrap();
        assert!(t.loading.is_none() && t.has_more && t.estimate == Some(5));
        assert_eq!(t.columns[0].name, "id");
        assert!(t.columns[0].is_primary_key);
    }

    #[test]
    fn ddl_retry_reemits_load_ddl_and_event_fills_text() {
        let mut s = state(&["a"]);
        let ws = s.open_workspace(s.config.connections[0].clone());
        s.open_table(ws, func(), RequestId(1));
        s.apply(Event::Notice {
            tab: TabId(1),
            text: "boom".into(),
        });
        let t = s.table_mut(TabId(1)).unwrap();
        assert!(!t.is_loading());
        assert!(matches!(
            t.retry(RequestId(2)),
            Command::LoadDdl {
                req: RequestId(2),
                ..
            }
        ));
        assert!(t.is_loading());
        s.apply(Event::Ddl {
            req: RequestId(2),
            text: "create".into(),
        });
        let t = s.table_mut(TabId(1)).unwrap();
        assert_eq!(t.ddl.as_deref(), Some("create"));
        assert!(!t.is_loading());
        t.notice = Some("stale".into());
        t.ddl_req = Some(RequestId(3));
        s.apply(Event::Ddl {
            req: RequestId(3),
            text: "create".into(),
        });
        assert!(s.table_mut(TabId(1)).unwrap().notice.is_none());
    }

    #[test]
    fn notice_and_crash_release_loading() {
        let mut s = state(&["a"]);
        let ws = s.open_workspace(s.config.connections[0].clone());
        s.open_table(ws, obj(), RequestId(1));
        s.apply(Event::Notice {
            tab: TabId(1),
            text: "boom".into(),
        });
        let t = s.table_mut(TabId(1)).unwrap();
        assert!(t.structure_req.is_none());
        assert_eq!(t.notice.as_deref(), Some("boom"));
        t.fetch(RequestId(2));
        s.apply(Event::ActorCrashed { tab: TabId(1) });
        assert!(s.table_mut(TabId(1)).unwrap().loading.is_none());
    }

    #[test]
    fn apply_connect_failed_keeps_form_open_with_error() {
        let mut s = state(&["a"]);
        let ws = s.open_workspace(s.config.connections[0].clone());
        s.apply(Event::ConnectFailed {
            ws,
            error: "refused".into(),
        });
        assert_eq!(s.workspaces[0].status, Status::Failed("refused".into()));
        assert!(s.workspaces[0].form_open());
    }

    #[test]
    fn query_tab_opens_and_run_sends_evaluate_only() {
        let mut s = state(&["a"]);
        let ws = s.open_workspace(s.config.connections[0].clone());
        let Some(Command::OpenEditor { tab, .. }) = s.open_query(ws, None) else {
            panic!("no OpenEditor");
        };
        s.workspace_mut(ws)
            .unwrap()
            .active_query_mut()
            .unwrap()
            .text = "select 1; select 2".into();
        let Some(Command::Evaluate { statements, .. }) =
            s.run_query(tab, 3..3, false, RequestId(5))
        else {
            panic!("no Evaluate");
        };
        assert_eq!(statements, vec!["select 1"]);
        // Pending run: a repeat is refused.
        assert!(s.run_query(tab, 0..0, true, RequestId(6)).is_none());
        assert!(s.take_commands().is_empty());
        // Once evaluated (Run verdict -> Execute from the dialog stack), the tab can run again.
        s.apply(Event::Evaluated {
            req: RequestId(5),
            evaluation: crustdb_core::safety::Evaluation {
                verdict: crustdb_core::safety::Verdict::Run,
                flags: vec![],
            },
        });
        assert!(s.run_query(tab, 0..0, true, RequestId(7)).is_some());
    }

    #[test]
    fn execute_marks_tab_running_and_routes_events_by_req() {
        let mut s = state(&["a"]);
        let ws = s.open_workspace(s.config.connections[0].clone());
        let Some(Command::OpenEditor { tab, .. }) = s.open_query(ws, None) else {
            panic!("no OpenEditor");
        };
        s.query_mut(tab).unwrap().text = "select 1".into();
        s.run_query(tab, 0..0, true, RequestId(5)).unwrap();
        s.apply(Event::Evaluated {
            req: RequestId(5),
            evaluation: crustdb_core::safety::Evaluation {
                verdict: crustdb_core::safety::Verdict::Run,
                flags: vec![],
            },
        });
        let cmd = s.take_commands().pop().unwrap();
        s.note_sent(&cmd);
        let Command::Execute { req, .. } = cmd else {
            panic!("no Execute");
        };
        assert_eq!(s.query_mut(tab).unwrap().running, Some(req));
        // A busy tab refuses a second run.
        assert!(s.run_query(tab, 0..0, true, RequestId(6)).is_none());
        s.apply(Event::Rows {
            req,
            rows: vec![],
            exhausted: true,
        });
        s.apply(Event::TxChanged {
            tab,
            state: crate::runtime::TxState::Open,
        });
        s.apply(Event::Notice {
            tab,
            text: "Connection lost — open transaction was rolled back by the server".into(),
        });
        let q = s.query_mut(tab).unwrap();
        assert_eq!(q.running, None);
        assert_eq!(q.tx, crate::runtime::TxState::Open);
        assert_eq!(q.pane.messages.len(), 1);
    }

    #[test]
    fn run_selection_is_in_chars_not_bytes() {
        let mut s = state(&["a"]);
        let ws = s.open_workspace(s.config.connections[0].clone());
        let Some(Command::OpenEditor { tab, .. }) = s.open_query(ws, None) else {
            panic!("no OpenEditor");
        };
        s.workspace_mut(ws)
            .unwrap()
            .active_query_mut()
            .unwrap()
            .text = "select 'ação🦀'; select 2".into();
        // chars 0..16 = `select 'ação🦀';`
        let Some(Command::Evaluate { statements, .. }) =
            s.run_query(tab, 0..16, false, RequestId(1))
        else {
            panic!("no Evaluate");
        };
        assert_eq!(statements, vec!["select 'ação🦀'"]);
    }

    #[test]
    fn delete_connection_requires_confirmation() {
        let mut s = state(&["a", "b"]);
        let id = s.config.connections[0].id;
        s.request_delete(id);
        assert_eq!(s.config.connections.len(), 2);
        assert!(matches!(
            s.dialogs.top(),
            Some(Dialog::DeleteConnection { id: i, .. }) if *i == id
        ));
        assert_eq!(s.confirm_delete(|_| Ok::<_, ()>(())), Ok(Some(id)));
        assert_eq!(s.config.connections.len(), 1);
        assert!(s.dialogs.top().is_none());
    }

    #[test]
    fn read_only_config_refuses_delete() {
        let mut s = state(&["a"]);
        s.config.read_only = true;
        s.request_delete(s.config.connections[0].id);
        assert!(s.dialogs.top().is_none());
        assert_eq!(s.config.connections.len(), 1);
    }

    #[test]
    fn failed_save_keeps_connection() {
        let mut s = state(&["a"]);
        s.request_delete(s.config.connections[0].id);
        assert_eq!(s.confirm_delete(|_| Err("io")), Err("io"));
        assert_eq!(s.config.connections.len(), 1);
    }

    #[test]
    fn evaluated_routes_by_registered_request() {
        use crustdb_core::safety::{Evaluation, Flag, FlagReason, Verdict};
        let ev = |verdict| Evaluation {
            verdict,
            flags: vec![Flag {
                index: 0,
                verdict,
                reason: FlagReason::NoWhere,
            }],
        };
        let mut s = state(&["a"]);
        let (ws, tab) = (WorkspaceId(5), TabId(6));
        s.register_evaluate(RequestId(1), ws, tab);
        s.apply(Event::Evaluated {
            req: RequestId(1),
            evaluation: ev(Verdict::Confirm),
        });
        assert!(matches!(
            s.dialogs.top(),
            Some(Dialog::Safety { ws: w, tab: t, req: RequestId(1), .. }) if *w == ws && *t == tab
        ));
        assert!(s.take_commands().is_empty());

        let mut s = state(&["a"]);
        s.apply(Event::Evaluated {
            req: RequestId(9),
            evaluation: ev(Verdict::Confirm),
        });
        assert!(s.dialogs.top().is_none() && s.take_commands().is_empty());

        s.register_evaluate(RequestId(2), ws, tab);
        s.apply(Event::Evaluated {
            req: RequestId(2),
            evaluation: Evaluation {
                verdict: Verdict::Run,
                flags: vec![],
            },
        });
        let cmds = s.take_commands();
        assert!(matches!(
            cmds[..],
            [Command::Execute {
                evaluation: RequestId(2),
                ..
            }]
        ));
        // Single use: a repeated answer is ignored.
        s.apply(Event::Evaluated {
            req: RequestId(2),
            evaluation: Evaluation {
                verdict: Verdict::Run,
                flags: vec![],
            },
        });
        assert!(s.take_commands().is_empty());
    }
}
