//! Workspace and table-tab actors behind the runtime router.
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crustdb_core::driver::{ConnectTarget, Driver};
use crustdb_core::model::{DriverKind, ObjectRef};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::{Mutex, watch};

use crate::runtime::{Command, DriverRegistry, Emitter, Event, RequestId, TabId, WorkspaceId};
use crate::storage::config::ConnectionConfig;

mod data;
mod editor;
pub(crate) use editor::{LOST, STALE_EVALUATION};
mod export;
mod workspace;

#[cfg(test)]
mod diagram_tests;
#[cfg(test)]
mod tests;

use data::DataActor;
use editor::EditorActor;
use workspace::{WorkspaceActor, WsMsg};

/// What a connected workspace shares with its table tabs.
pub(crate) struct Shared {
    pub(crate) driver: Arc<dyn Driver>,
    pub(crate) target: ConnectTarget,
    pub(crate) config: ConnectionConfig,
}

impl Shared {
    /// Target for a session on `db`. SQLite keeps the configured file: `db` is a schema
    /// (`main`) there, not something to connect to.
    pub(crate) fn target_for(&self, db: &str) -> ConnectTarget {
        let mut t = self.target.clone();
        if self.driver.kind() != DriverKind::Sqlite {
            t.database = Some(db.to_string());
        }
        t
    }
}

/// `None` until the workspace is connected.
pub(crate) type SharedRx = watch::Receiver<Option<Arc<Shared>>>;

/// Runs an actor under a supervisor: a panic emits `ActorCrashed` and restarts it with a fresh
/// state on the same mailbox (the message being handled is lost). Ends when the mailbox closes.
fn supervise<M, F, Fut>(tab: TabId, out: Emitter, rx: UnboundedReceiver<M>, mk: F)
where
    M: Send + 'static,
    F: Fn(Arc<Mutex<UnboundedReceiver<M>>>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        let rx = Arc::new(Mutex::new(rx));
        while let Err(e) = tokio::spawn(mk(rx.clone())).await {
            if !e.is_panic() {
                break;
            }
            out.emit(Event::ActorCrashed { tab });
        }
    });
}

struct WsEntry {
    tx: UnboundedSender<WsMsg>,
    shared: SharedRx,
}

struct TabEntry {
    ws: WorkspaceId,
    editor: bool,
    tx: UnboundedSender<Command>,
}

/// Routes commands to workspace and table-tab actors.
pub(crate) struct Actors {
    drivers: DriverRegistry,
    out: Emitter,
    ws: HashMap<WorkspaceId, WsEntry>,
    tabs: HashMap<TabId, TabEntry>,
    /// Mailbox of the task running each export, for `CancelExport`.
    exports: HashMap<RequestId, UnboundedSender<Command>>,
}

impl Actors {
    pub(crate) fn new(drivers: DriverRegistry, out: Emitter) -> Self {
        Self {
            drivers,
            out,
            ws: HashMap::new(),
            tabs: HashMap::new(),
            exports: HashMap::new(),
        }
    }

    /// Returns the command back when no actor handles it.
    pub(crate) fn dispatch(&mut self, cmd: Command) -> Option<Command> {
        // Editor actors end themselves on a successful close; forget them.
        self.tabs.retain(|_, t| !t.tx.is_closed());
        self.exports.retain(|_, tx| !tx.is_closed());
        match cmd {
            Command::Connect {
                ws,
                config,
                secrets,
            } => {
                self.connect(
                    ws,
                    Command::Connect {
                        ws,
                        config,
                        secrets,
                    },
                );
                None
            }
            Command::Disconnect { ws } => {
                self.tabs.retain(|_, t| t.ws != ws);
                match self.ws.remove(&ws) {
                    Some(e) => {
                        let _ = e.tx.send(WsMsg::Disconnect);
                        None
                    }
                    None => Some(Command::Disconnect { ws }),
                }
            }
            Command::HostKeyDecision { ws, .. }
            | Command::LoadChildren { ws, .. }
            | Command::LoadColumns { ws, .. }
            | Command::LoadDiagram { ws, .. } => match self.ws.get(&ws) {
                Some(e) => {
                    let _ = e.tx.send(WsMsg::Cmd(cmd));
                    None
                }
                None => Some(cmd),
            },
            Command::OpenTable { ws, tab, object } => {
                match self.ws.get(&ws).map(|e| e.shared.clone()) {
                    Some(shared) => self.open_table(ws, tab, object, shared),
                    None => self.out.emit(Event::Notice {
                        tab,
                        text: "workspace is not connected".into(),
                    }),
                }
                None
            }
            Command::OpenEditor { ws, tab, database } => {
                match self.ws.get(&ws).map(|e| e.shared.clone()) {
                    Some(shared) => self.open_editor(ws, tab, database, shared),
                    None => self.out.emit(Event::Notice {
                        tab,
                        text: "workspace is not connected".into(),
                    }),
                }
                None
            }
            Command::CloseTab { tab, resolve: None }
                if self.tabs.get(&tab).is_some_and(|t| !t.editor) =>
            {
                self.tabs.remove(&tab);
                None
            }
            Command::ExportTable {
                tab,
                req,
                query,
                path,
            } => {
                let shared = self
                    .tabs
                    .get(&tab)
                    .filter(|t| !t.editor)
                    .and_then(|t| self.ws.get(&t.ws))
                    .map(|e| e.shared.clone());
                match shared {
                    Some(shared) => self.export_table(export::TableJob {
                        tab,
                        req,
                        query,
                        path,
                        shared,
                        out: self.out.clone(),
                    }),
                    None => self.export_failed(req, "table tab is not open"),
                }
                None
            }
            Command::ExportResult { tab, req, path } => {
                match self.tabs.get(&tab).filter(|t| t.editor) {
                    Some(t) => {
                        self.exports.insert(req, t.tx.clone());
                        let _ = t.tx.send(Command::ExportResult { tab, req, path });
                    }
                    None => self.export_failed(req, "query tab is not open"),
                }
                None
            }
            Command::CancelExport { req } => {
                if let Some(tx) = self.exports.get(&req) {
                    let _ = tx.send(Command::CancelExport { req });
                }
                None
            }
            Command::FetchPage { tab, .. }
            | Command::LoadStructure { tab, .. }
            | Command::LoadDdl { tab, .. }
            | Command::Evaluate { tab, .. }
            | Command::Execute { tab, .. }
            | Command::LoadMore { tab, .. }
            | Command::Tx { tab, .. }
            | Command::UseDatabase { tab, .. }
            | Command::CloseTab { tab, .. }
            | Command::CancelTab { tab } => match self.tabs.get(&tab) {
                Some(t) => {
                    let _ = t.tx.send(cmd);
                    None
                }
                None => Some(cmd),
            },
        }
    }

    fn connect(&mut self, ws: WorkspaceId, connect: Command) {
        let Command::Connect { config, .. } = &connect else {
            return;
        };
        let Some(driver) = self.drivers.get(config.driver) else {
            self.out.emit(Event::ConnectFailed {
                ws,
                error: "no driver for this connection".into(),
            });
            return;
        };
        // The old actor holds a sender to its own mailbox, so it must be told to stop.
        if let Some(old) = self.ws.remove(&ws) {
            let _ = old.tx.send(WsMsg::Disconnect);
        }
        self.tabs.retain(|_, t| t.ws != ws);
        let (tx, rx) = unbounded_channel();
        let (shared_tx, shared) = watch::channel(None);
        let shared_tx = Arc::new(shared_tx);
        let _ = tx.send(WsMsg::Cmd(connect));
        let (out, self_tx) = (self.out.clone(), tx.clone());
        let first_run = AtomicBool::new(true);
        supervise(TabId(0), self.out.clone(), rx, move |rx| {
            if !first_run.swap(false, Ordering::SeqCst) {
                // Restarted after a panic: the old connection state is gone.
                shared_tx.send_replace(None);
                out.emit(Event::ConnectFailed {
                    ws,
                    error: "workspace actor crashed".into(),
                });
            }
            WorkspaceActor::new(
                ws,
                driver.clone(),
                out.clone(),
                self_tx.clone(),
                shared_tx.clone(),
            )
            .run(rx)
        });
        self.ws.insert(ws, WsEntry { tx, shared });
    }

    fn export_failed(&self, req: RequestId, why: &str) {
        self.out.emit(Event::ExportDone {
            req,
            result: Err(why.into()),
        });
    }

    fn export_table(&mut self, job: export::TableJob) {
        let (req, out) = (job.req, self.out.clone());
        let (tx, rx) = unbounded_channel();
        self.exports.insert(req, tx);
        let task = tokio::spawn(export::table(job, rx));
        tokio::spawn(async move {
            if task.await.is_err() {
                out.emit(Event::ExportDone {
                    req,
                    result: Err("export task crashed".into()),
                });
            }
        });
    }

    fn open_table(&mut self, ws: WorkspaceId, tab: TabId, object: ObjectRef, shared: SharedRx) {
        let (tx, rx) = unbounded_channel();
        let out = self.out.clone();
        supervise(tab, self.out.clone(), rx, move |rx| {
            DataActor::new(tab, object.clone(), shared.clone(), out.clone()).run(rx)
        });
        self.tabs.insert(
            tab,
            TabEntry {
                ws,
                editor: false,
                tx,
            },
        );
    }

    fn open_editor(
        &mut self,
        ws: WorkspaceId,
        tab: TabId,
        database: Option<String>,
        shared: SharedRx,
    ) {
        let (tx, rx) = unbounded_channel();
        let out = self.out.clone();
        supervise(tab, self.out.clone(), rx, move |rx| {
            EditorActor::new(tab, database.clone(), shared.clone(), out.clone()).run(rx)
        });
        self.tabs.insert(
            tab,
            TabEntry {
                ws,
                editor: true,
                tx,
            },
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod target_tests {
    use std::time::Duration;

    use crustdb_core::driver::ConnectTarget;
    use crustdb_core::model::DriverKind;
    use crustdb_testkit::fake_driver::{FakeDriver, FakeScript};

    use super::Shared;
    use super::tests::config;

    fn shared(kind: DriverKind) -> Shared {
        let config = config(Some("/tmp/a.db"));
        Shared {
            driver: FakeDriver::with_kind(FakeScript::default(), kind),
            target: ConnectTarget {
                host: String::new(),
                port: 0,
                connect_addr: None,
                user: String::new(),
                password: None,
                database: config.database.clone(),
                tls: config.tls.clone(),
                connect_timeout: Duration::from_secs(1),
            },
            config,
        }
    }

    #[test]
    fn sqlite_target_keeps_file_path() {
        // Act & Assert
        let t = shared(DriverKind::Sqlite).target_for("main");
        assert_eq!(t.database.as_deref(), Some("/tmp/a.db"));
        let t = shared(DriverKind::Mysql).target_for("main");
        assert_eq!(t.database.as_deref(), Some("main"));
    }
}
