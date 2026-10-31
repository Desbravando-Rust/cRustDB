use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::time::Instant;

use crustdb_core::model::{ConnectionId, ObjectRef, ServerVersion};
use secrecy::SecretString;

use super::autocomplete::ColumnCache;
use super::dialogs::{Dialog, DialogStack};
use super::query_tab::{QueryTab, char_to_byte};
use super::table_tab::{Structure, TableTab, TableView};
use super::tree::{ObjectTree, OpenAction, open_action};
use crate::runtime::{Command, Event, RequestId, TabId, TxState, WorkspaceId};
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
    pub diagram: Option<super::diagram::Diagram>,
    /// The password this session connected with, for the safety prompt (never read from the keyring).
    pub session_password: Option<SecretString>,
    /// Key tree and value viewer; only for key-value data models.
    pub keyspace: Option<super::keyspace::Keyspace>,
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
    /// Commands that (re)load the tables folder of the configured database; idempotent.
    pub fn expand_home(&mut self) -> Vec<Command> {
        match self.config.database.clone() {
            Some(db) if self.capabilities().table_tab => self.tree.expand_tables_of(&db),
            _ => vec![],
        }
    }

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
    pub(super) evaluating: HashMap<RequestId, (WorkspaceId, TabId)>,
    pub(super) next_exec: u64,
    pub(super) outbox: Vec<Command>,
    /// Workspaces waiting for their editors' COMMIT/ROLLBACK before the real `Disconnect`.
    pub(super) disconnecting: HashMap<WorkspaceId, HashSet<TabId>>,
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
            disconnecting: HashMap::new(),
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
            .filter(|w| w.capabilities().table_tab)
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
        if !w.capabilities().sql_editor {
            return None;
        }
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
                let mut cmds = vec![];
                if let Some(w) = self.workspace_mut(ws) {
                    w.status = Status::Online(server);
                    w.tree.reset();
                    cmds = w.expand_home();
                }
                self.outbox.extend(cmds);
                self.start_keyspace(ws);
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
            Event::Diagram { req, tables } => {
                let mut open = self
                    .workspaces
                    .iter_mut()
                    .filter_map(|w| w.diagram.as_mut());
                if let Some(d) = open.find(|d| d.req == req) {
                    d.apply(tables);
                }
            }
            Event::Children { req, items } => {
                for w in &mut self.workspaces {
                    w.tree.apply_children(req, items.clone());
                }
            }
            Event::ChildrenFailed { req, error } => {
                for w in &mut self.workspaces {
                    w.tree.fail_children(req, error.clone());
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
            Event::CellUpdated { req, error } => {
                for Tab::Table(t) in self.workspaces.iter_mut().flat_map(|w| w.tabs.iter_mut()) {
                    t.cell_updated(req, error.clone());
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
                // A notice while resolving means the COMMIT/ROLLBACK failed: stay connected.
                self.disconnecting.retain(|_, tabs| !tabs.contains(&tab));
                if let Some(t) = self.table_mut(tab) {
                    t.fail(text);
                } else if let Some(q) = self.query_mut(tab) {
                    q.apply(&Event::Notice { tab, text });
                }
            }
            Event::ResultStarted { req, .. }
            | Event::Rows { req, .. }
            | Event::FetchDone { req }
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
            Event::TxChanged { tab, state } => {
                if let Some(q) = self.query_mut(tab) {
                    q.apply(&e);
                }
                if state == TxState::Idle {
                    self.tx_resolved(tab);
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
            _ => self.apply_keyspace(&e),
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

#[path = "workspace_open.rs"]
mod open;

#[cfg(test)]
#[path = "workspace_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "workspace_open_tests.rs"]
mod open_tests;
