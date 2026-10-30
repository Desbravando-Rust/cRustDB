//! Owns one store (key-value) connection: a single session, cancelled by dropping it.
use std::collections::VecDeque;
use std::future::Future;
use std::sync::Arc;

use crustdb_core::driver::{ConnectTarget, SessionPurpose};
use crustdb_core::error::{DriverError, ErrorKind};
use crustdb_core::store::keyspace::EntryKey;
use crustdb_core::store::{EntryView, ScanRequest, StoreDriver, StoreSession};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::{Mutex, oneshot};

use super::tunnel::tunnel_target;
use super::workspace::WsMsg;
use crate::net::ssh::Tunnel;
use crate::runtime::{
    Command, Emitter, Event, ROWS_BATCH, RequestId, ResolvedSecrets, TabId, WorkspaceId,
};
use crate::storage::config::ConnectionConfig;

/// Workspace-level problems have no tab; they are reported on the tab 0 sentinel.
const NO_TAB: TabId = TabId(0);

type Rx = UnboundedReceiver<WsMsg>;
type Session = Box<dyn StoreSession>;

pub(crate) struct StoreActor {
    ws: WorkspaceId,
    driver: Arc<dyn StoreDriver>,
    out: Emitter,
    self_tx: UnboundedSender<WsMsg>,
    /// Where to reconnect after a cancel or a dropped session.
    target: Option<ConnectTarget>,
    /// Entries per `read`.
    read_limit: usize,
    /// `None` while idle after a cancel; the next operation reconnects.
    session: Option<Session>,
    /// Kept so the SSH tunnel outlives the sessions that use it.
    tunnel: Option<Tunnel>,
    decision: Option<oneshot::Sender<bool>>,
    /// Messages that arrived while an operation was in flight.
    queue: VecDeque<WsMsg>,
    /// The request whose operation is running; only a `CancelStore` naming it may abort it.
    in_flight: Option<RequestId>,
    stopped: bool,
}

impl StoreActor {
    pub(crate) fn new(
        ws: WorkspaceId,
        driver: Arc<dyn StoreDriver>,
        out: Emitter,
        self_tx: UnboundedSender<WsMsg>,
    ) -> Self {
        Self {
            ws,
            driver,
            out,
            self_tx,
            target: None,
            read_limit: ROWS_BATCH,
            session: None,
            tunnel: None,
            decision: None,
            queue: VecDeque::new(),
            in_flight: None,
            stopped: false,
        }
    }

    pub(crate) async fn run(mut self, rx: Arc<Mutex<Rx>>) {
        let mut rx = rx.lock().await;
        while !self.stopped {
            let m = match self.queue.pop_front() {
                Some(m) => m,
                None => match rx.recv().await {
                    Some(m) => m,
                    None => break,
                },
            };
            self.handle(m, &mut rx).await;
        }
        if let Some(s) = self.session.take() {
            s.close().await;
        }
    }

    async fn handle(&mut self, m: WsMsg, rx: &mut Rx) {
        self.in_flight = match &m {
            WsMsg::Cmd(
                Command::ListNamespaces { req, .. }
                | Command::ScanKeys { req, .. }
                | Command::ReadEntry { req, .. },
            ) => Some(*req),
            _ => None,
        };
        let ws = self.ws;
        match m {
            WsMsg::Cmd(Command::Connect {
                config, secrets, ..
            }) => self.connect(config, secrets, rx).await,
            WsMsg::Cmd(Command::ListNamespaces { req, .. }) => {
                let result = self
                    .with_session(rx, |mut s| async move {
                        let r = s.namespaces().await;
                        (s, r)
                    })
                    .await;
                self.out.emit(Event::Namespaces { req, result });
            }
            WsMsg::Cmd(Command::ScanKeys {
                req,
                cursor,
                pattern,
                ..
            }) => {
                let scan = ScanRequest {
                    cursor,
                    pattern,
                    count: ROWS_BATCH as u32,
                };
                let result = self
                    .with_session(rx, |mut s| async move {
                        let r = s.scan(&scan).await;
                        (s, r)
                    })
                    .await;
                self.out.emit(Event::KeysScanned { req, result });
            }
            WsMsg::Cmd(Command::ReadEntry { req, key, .. }) => {
                let result = self.read(rx, key).await;
                self.out.emit(Event::EntryRead { req, result });
            }
            WsMsg::Cmd(Command::LoadChildren { req, .. }) => {
                self.out.emit(Event::Notice {
                    tab: NO_TAB,
                    text: "object tree is not available for this connection".into(),
                });
                // The sidebar leaves its pending state on any answer.
                self.out.emit(Event::Children { req, items: vec![] });
            }
            WsMsg::Cmd(Command::LoadColumns { req, .. }) => {
                self.out.emit(Event::Columns { req, columns: None });
            }
            WsMsg::Cmd(Command::LoadDiagram { req, .. }) => {
                self.out.emit(Event::Diagram { req, tables: None });
            }
            WsMsg::Cmd(Command::HostKeyDecision { accept, .. }) => self.decide(accept),
            WsMsg::AskHostKey(tx) => self.decision = Some(tx),
            WsMsg::Disconnect => self.stopped = true,
            // `CancelStore` with nothing in flight, and messages SQL actors own.
            WsMsg::Cmd(_) | WsMsg::Connected(_) => {
                tracing::debug!("store workspace {ws:?} ignored a message");
            }
        }
    }

    fn decide(&mut self, accept: bool) {
        if let Some(tx) = self.decision.take() {
            let _ = tx.send(accept);
        }
    }

    async fn connect(&mut self, config: ConnectionConfig, secrets: ResolvedSecrets, rx: &mut Rx) {
        self.session = None;
        self.tunnel = None;
        self.target = None;
        self.read_limit = config.page_size as usize;
        let (driver, out, ws, tx) = (
            self.driver.clone(),
            self.out.clone(),
            self.ws,
            self.self_tx.clone(),
        );
        let opening = async move {
            let (tunnel, target) = tunnel_target(&config, secrets, ws, &out, &tx).await?;
            let session = driver
                .connect(&target, SessionPurpose::Data)
                .await
                .map_err(|e| e.message)?;
            Ok::<_, String>((tunnel, target, session))
        };
        match self.race(rx, opening).await {
            Some(Ok((tunnel, target, session))) => {
                let server = session.server_version().clone();
                (self.tunnel, self.target, self.session) = (tunnel, Some(target), Some(session));
                self.out.emit(Event::Connected { ws, server });
            }
            Some(Err(error)) => self.out.emit(Event::ConnectFailed { ws, error }),
            None if !self.stopped => self.out.emit(Event::ConnectFailed {
                ws,
                error: "cancelled".into(),
            }),
            None => {}
        }
    }

    async fn read(&mut self, rx: &mut Rx, key: EntryKey) -> Result<EntryView, String> {
        let limit = self.read_limit;
        self.with_session(rx, |mut s| async move {
            let r = s.read(&key, limit).await;
            (s, r)
        })
        .await
    }

    /// Runs `op` on the session, reconnecting first when there is none. The operation owns the
    /// session while in flight, so cancelling means dropping the future. A session that failed
    /// with a connection-level error is not put back.
    // ponytail: operations are serial per workspace; a read waits behind an in-flight scan.
    // A second session would lift this.
    async fn with_session<T, F, Fut>(&mut self, rx: &mut Rx, op: F) -> Result<T, String>
    where
        F: FnOnce(Session) -> Fut,
        Fut: Future<Output = (Session, Result<T, DriverError>)>,
    {
        let session = match self.session.take() {
            Some(s) => s,
            None => self.reopen(rx).await?,
        };
        match self.race(rx, op(session)).await {
            Some((s, Ok(v))) => {
                self.session = Some(s);
                Ok(v)
            }
            Some((s, Err(e))) => {
                if !matches!(e.kind, ErrorKind::Connect | ErrorKind::Protocol) {
                    self.session = Some(s);
                }
                Err(e.message)
            }
            None => Err("cancelled".into()),
        }
    }

    async fn reopen(&mut self, rx: &mut Rx) -> Result<Session, String> {
        let target = self
            .target
            .clone()
            .ok_or_else(|| "workspace is not connected".to_string())?;
        let driver = self.driver.clone();
        let connecting = async move {
            driver
                .connect(&target, SessionPurpose::Data)
                .await
                .map_err(|e| e.message)
        };
        self.race(rx, connecting)
            .await
            .unwrap_or_else(|| Err("cancelled".into()))
    }

    /// Awaits `op` while serving the mailbox. `None` means it was abandoned (`CancelStore`,
    /// `Disconnect` or a closed mailbox) and `op` was dropped with everything it owned.
    async fn race<T>(&mut self, rx: &mut Rx, op: impl Future<Output = T>) -> Option<T> {
        tokio::pin!(op);
        loop {
            tokio::select! {
                r = &mut op => return Some(r),
                m = rx.recv() => match m {
                    None | Some(WsMsg::Disconnect) => {
                        self.stopped = true;
                        return None;
                    }
                    Some(WsMsg::Cmd(Command::CancelStore { req, .. })) => {
                        if self.in_flight == Some(req) {
                            return None;
                        }
                        self.cancel_queued(req);
                    }
                    Some(WsMsg::Cmd(Command::HostKeyDecision { accept, .. })) => self.decide(accept),
                    Some(WsMsg::AskHostKey(tx)) => self.decision = Some(tx),
                    Some(other) => self.queue.push_back(other),
                },
            }
        }
    }

    /// Answers "cancelled" for a still-queued request; unknown or finished ones are ignored.
    fn cancel_queued(&mut self, req: RequestId) {
        let queued = |m: &WsMsg| match m {
            WsMsg::Cmd(
                Command::ListNamespaces { req: r, .. }
                | Command::ScanKeys { req: r, .. }
                | Command::ReadEntry { req: r, .. },
            ) => *r == req,
            _ => false,
        };
        let Some(i) = self.queue.iter().position(queued) else {
            return;
        };
        match self.queue.remove(i) {
            Some(WsMsg::Cmd(Command::ListNamespaces { req, .. })) => {
                self.out.emit(Event::Namespaces {
                    req,
                    result: Err("cancelled".into()),
                })
            }
            Some(WsMsg::Cmd(Command::ScanKeys { req, .. })) => self.out.emit(Event::KeysScanned {
                req,
                result: Err("cancelled".into()),
            }),
            Some(WsMsg::Cmd(Command::ReadEntry { req, .. })) => self.out.emit(Event::EntryRead {
                req,
                result: Err("cancelled".into()),
            }),
            _ => {}
        }
    }
}
