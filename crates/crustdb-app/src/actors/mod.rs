//! Workspace and table-tab actors behind the runtime router.
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crustdb_core::driver::{ConnectTarget, Driver};
use crustdb_core::model::ObjectRef;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::{Mutex, watch};

use crate::runtime::{Command, DriverRegistry, Emitter, Event, TabId, WorkspaceId};
use crate::storage::config::ConnectionConfig;

mod data;
mod workspace;

#[cfg(test)]
mod tests;

use data::DataActor;
use workspace::{WorkspaceActor, WsMsg};

/// What a connected workspace shares with its table tabs.
pub(crate) struct Shared {
    pub(crate) driver: Arc<dyn Driver>,
    pub(crate) target: ConnectTarget,
    pub(crate) config: ConnectionConfig,
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
    tx: UnboundedSender<Command>,
}

/// Routes commands to workspace and table-tab actors.
pub(crate) struct Actors {
    drivers: DriverRegistry,
    out: Emitter,
    ws: HashMap<WorkspaceId, WsEntry>,
    tabs: HashMap<TabId, TabEntry>,
}

impl Actors {
    pub(crate) fn new(drivers: DriverRegistry, out: Emitter) -> Self {
        Self {
            drivers,
            out,
            ws: HashMap::new(),
            tabs: HashMap::new(),
        }
    }

    /// Returns the command back when no actor handles it.
    pub(crate) fn dispatch(&mut self, cmd: Command) -> Option<Command> {
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
            Command::HostKeyDecision { ws, .. } | Command::LoadChildren { ws, .. } => {
                match self.ws.get(&ws) {
                    Some(e) => {
                        let _ = e.tx.send(WsMsg::Cmd(cmd));
                        None
                    }
                    None => Some(cmd),
                }
            }
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
            Command::CloseTab { tab, resolve: None } if self.tabs.remove(&tab).is_some() => None,
            Command::FetchPage { tab, .. }
            | Command::LoadStructure { tab, .. }
            | Command::LoadDdl { tab, .. }
            | Command::CancelTab { tab } => match self.tabs.get(&tab) {
                Some(t) => {
                    let _ = t.tx.send(cmd);
                    None
                }
                None => Some(cmd),
            },
            other => Some(other),
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

    fn open_table(&mut self, ws: WorkspaceId, tab: TabId, object: ObjectRef, shared: SharedRx) {
        let (tx, rx) = unbounded_channel();
        let out = self.out.clone();
        supervise(tab, self.out.clone(), rx, move |rx| {
            DataActor::new(tab, object.clone(), shared.clone(), out.clone()).run(rx)
        });
        self.tabs.insert(tab, TabEntry { ws, tx });
    }
}
