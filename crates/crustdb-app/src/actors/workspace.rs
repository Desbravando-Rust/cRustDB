//! Owns one connection: SSH tunnel, per-database Metadata sessions, ping and reconnect.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crustdb_core::driver::{ConnectTarget, Driver, Session, SessionPurpose};
use crustdb_core::error::{DriverError, ErrorKind};
use crustdb_core::model::{DriverKind, ObjectKind, ObjectRef};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::{Mutex, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, Interval, MissedTickBehavior, interval_at, sleep_until};

use super::Shared;
use crate::net::known_hosts::KnownHosts;
use crate::net::ssh::{HostKeyAsk, Tunnel, open_tunnel};
use crate::runtime::{Command, Emitter, Event, RequestId, ResolvedSecrets, TabId, WorkspaceId};
use crate::storage::config::ConnectionConfig;
use crate::storage::paths::config_dir;

const PING_EVERY: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Workspace-level problems have no tab; they are reported on the tab 0 sentinel.
const NO_TAB: TabId = TabId(0);

/// Wait before reconnect attempt `attempt` (0-based): 1, 2, 4, 8 s ... capped at 30 s.
pub(crate) fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(1u64.checked_shl(attempt).unwrap_or(u64::MAX).min(30))
}

// Short-lived mailbox messages; boxing `Command` would only add an allocation per send.
#[allow(clippy::large_enum_variant)]
pub(crate) enum WsMsg {
    Cmd(Command),
    /// From the connect task: where to deliver the user's host key decision.
    AskHostKey(oneshot::Sender<bool>),
    Connected(Result<Ready, String>),
    Disconnect,
}

pub(crate) struct Ready {
    tunnel: Option<Tunnel>,
    session: Box<dyn Session>,
    shared: Arc<Shared>,
}

pub(crate) struct WorkspaceActor {
    ws: WorkspaceId,
    driver: Arc<dyn Driver>,
    out: Emitter,
    self_tx: UnboundedSender<WsMsg>,
    shared_tx: Arc<watch::Sender<Option<Arc<Shared>>>>,
    shared: Option<Arc<Shared>>,
    /// Metadata sessions by database (PG: one per database; MySQL: only `home`).
    sessions: HashMap<String, Box<dyn Session>>,
    home: String,
    tunnel: Option<Tunnel>,
    connecting: Option<JoinHandle<()>>,
    decision: Option<oneshot::Sender<bool>>,
    down: Vec<String>,
    attempt: u32,
    retry_at: Option<Instant>,
    ping: Interval,
}

impl WorkspaceActor {
    pub(crate) fn new(
        ws: WorkspaceId,
        driver: Arc<dyn Driver>,
        out: Emitter,
        self_tx: UnboundedSender<WsMsg>,
        shared_tx: Arc<watch::Sender<Option<Arc<Shared>>>>,
    ) -> Self {
        let mut ping = interval_at(Instant::now() + PING_EVERY, PING_EVERY);
        ping.set_missed_tick_behavior(MissedTickBehavior::Delay);
        Self {
            ws,
            driver,
            out,
            self_tx,
            shared_tx,
            shared: None,
            sessions: HashMap::new(),
            home: String::new(),
            tunnel: None,
            connecting: None,
            decision: None,
            down: Vec::new(),
            attempt: 0,
            retry_at: None,
            ping,
        }
    }

    pub(crate) async fn run(mut self, rx: Arc<Mutex<UnboundedReceiver<WsMsg>>>) {
        let mut rx = rx.lock().await;
        loop {
            tokio::select! {
                m = rx.recv() => match m {
                    None | Some(WsMsg::Disconnect) => break,
                    Some(m) => self.handle(m).await,
                },
                _ = self.ping.tick(), if self.shared.is_some() => self.ping_all().await,
                () = sleep_until(self.retry_at.unwrap_or_else(Instant::now)), if self.retry_at.is_some() => {
                    self.reconnect().await;
                }
            }
        }
        if let Some(c) = self.connecting.take() {
            c.abort();
        }
        for (_, s) in self.sessions.drain() {
            s.close().await;
        }
    }

    async fn handle(&mut self, m: WsMsg) {
        match m {
            WsMsg::Cmd(Command::Connect {
                config, secrets, ..
            }) => self.start_connect(config, secrets),
            WsMsg::Cmd(Command::HostKeyDecision { accept, .. }) => {
                if let Some(tx) = self.decision.take() {
                    let _ = tx.send(accept);
                }
            }
            WsMsg::Cmd(Command::LoadChildren {
                req,
                database,
                schema,
                kind,
                ..
            }) => self.load_children(req, database, schema, kind).await,
            WsMsg::Cmd(_) | WsMsg::Disconnect => {}
            WsMsg::AskHostKey(tx) => self.decision = Some(tx),
            WsMsg::Connected(Ok(ready)) => self.on_connected(ready),
            WsMsg::Connected(Err(error)) => {
                self.out.emit(Event::ConnectFailed { ws: self.ws, error })
            }
        }
    }

    fn start_connect(&mut self, config: ConnectionConfig, secrets: ResolvedSecrets) {
        if let Some(c) = self.connecting.take() {
            c.abort();
        }
        let (driver, out, ws, tx) = (
            self.driver.clone(),
            self.out.clone(),
            self.ws,
            self.self_tx.clone(),
        );
        self.connecting = Some(tokio::spawn(async move {
            let res = open(driver, config, secrets, ws, &out, &tx).await;
            let _ = tx.send(WsMsg::Connected(res));
        }));
    }

    fn on_connected(&mut self, r: Ready) {
        self.connecting = None;
        self.home = r.shared.config.database.clone().unwrap_or_default();
        let server = r.session.server_version().clone();
        self.sessions.insert(self.home.clone(), r.session);
        self.tunnel = r.tunnel;
        self.shared = Some(r.shared.clone());
        self.shared_tx.send_replace(Some(r.shared));
        self.ping.reset();
        self.out.emit(Event::Connected {
            ws: self.ws,
            server,
        });
    }

    /// A dead SSH tunnel cannot be revived here: stop retrying and tell the UI.
    fn tunnel_dead(&mut self) -> bool {
        if !self.tunnel.as_ref().is_some_and(Tunnel::is_closed) {
            return false;
        }
        self.sessions.clear();
        self.down.clear();
        self.retry_at = None;
        self.shared = None;
        self.shared_tx.send_replace(None);
        self.out.emit(Event::ConnectFailed {
            ws: self.ws,
            error: "SSH tunnel closed".into(),
        });
        true
    }

    async fn ping_all(&mut self) {
        if self.tunnel_dead() {
            return;
        }
        let mut failed = Vec::new();
        for (key, s) in &mut self.sessions {
            match s.ping().await {
                Ok(latency) if *key == self.home => self.out.emit(Event::Ping {
                    ws: self.ws,
                    latency,
                }),
                Ok(_) => {}
                Err(_) => failed.push(key.clone()),
            }
        }
        for key in failed {
            self.mark_down(key);
        }
    }

    fn mark_down(&mut self, key: String) {
        self.sessions.remove(&key);
        if !self.down.contains(&key) {
            self.down.push(key);
        }
        if self.retry_at.is_none() {
            self.out.emit(Event::Reconnecting { ws: self.ws });
            self.attempt = 0;
            self.retry_at = Some(Instant::now() + backoff(0));
        }
    }

    async fn reconnect(&mut self) {
        if self.tunnel_dead() {
            return;
        }
        for key in std::mem::take(&mut self.down) {
            match self.open_metadata(&key).await {
                Ok(s) => {
                    self.sessions.insert(key, s);
                }
                Err(_) => self.down.push(key),
            }
        }
        if self.down.is_empty() {
            self.retry_at = None;
            if let Some(s) = self.sessions.get(&self.home) {
                let server = s.server_version().clone();
                self.out.emit(Event::Connected {
                    ws: self.ws,
                    server,
                });
            }
        } else {
            self.attempt += 1;
            self.retry_at = Some(Instant::now() + backoff(self.attempt));
        }
    }

    async fn open_metadata(&mut self, key: &str) -> Result<Box<dyn Session>, DriverError> {
        let Some(sh) = self.shared.clone() else {
            return Err(not_connected());
        };
        let mut target = sh.target.clone();
        if !key.is_empty() {
            target.database = Some(key.to_string());
        }
        sh.driver.connect(&target, SessionPurpose::Metadata).await
    }

    async fn load_children(
        &mut self,
        req: RequestId,
        database: String,
        schema: Option<String>,
        kind: Option<ObjectKind>,
    ) {
        match self.children(&database, schema.as_deref(), kind).await {
            Ok(items) => self.out.emit(Event::Children { req, items }),
            Err((key, e)) => {
                if matches!(e.kind, ErrorKind::Connect | ErrorKind::Protocol) {
                    self.mark_down(key);
                }
                self.out.emit(Event::Notice {
                    tab: NO_TAB,
                    text: e.message,
                });
            }
        }
    }

    /// `None`/`Database` lists databases; `Schema` lists schemas of `database`.
    async fn children(
        &mut self,
        database: &str,
        schema: Option<&str>,
        kind: Option<ObjectKind>,
    ) -> Result<Vec<ObjectRef>, (String, DriverError)> {
        let Some(sh) = self.shared.clone() else {
            return Err((String::new(), not_connected()));
        };
        let listing_dbs = matches!(kind, None | Some(ObjectKind::Database));
        let key =
            if sh.driver.kind() == DriverKind::Postgres && !database.is_empty() && !listing_dbs {
                database.to_string()
            } else {
                self.home.clone()
            };
        if !self.sessions.contains_key(&key) {
            let s = self
                .open_metadata(&key)
                .await
                .map_err(|e| (key.clone(), e))?;
            self.sessions.insert(key.clone(), s);
        }
        let Some(s) = self.sessions.get_mut(&key) else {
            return Err((key, not_connected()));
        };
        let intro = sh.driver.introspector();
        let obj = |db: &str, schema: Option<&str>, name: String, kind| ObjectRef {
            database: db.to_string(),
            schema: schema.map(String::from),
            name,
            kind,
        };
        let items = match kind {
            None | Some(ObjectKind::Database) => intro.databases(s.as_mut()).await.map(|v| {
                v.into_iter()
                    .map(|n| obj(&n, None, n.clone(), ObjectKind::Database))
                    .collect()
            }),
            Some(ObjectKind::Schema) => intro.schemas(s.as_mut(), database).await.map(|v| {
                v.into_iter()
                    .map(|n| obj(database, Some(&n), n.clone(), ObjectKind::Schema))
                    .collect()
            }),
            Some(k) => intro.objects(s.as_mut(), database, schema, k).await,
        };
        items.map_err(|e| (key, e))
    }
}

fn not_connected() -> DriverError {
    DriverError {
        kind: ErrorKind::Connect,
        message: "workspace is not connected".into(),
    }
}

async fn open(
    driver: Arc<dyn Driver>,
    config: ConnectionConfig,
    secrets: ResolvedSecrets,
    ws: WorkspaceId,
    out: &Emitter,
    tx: &UnboundedSender<WsMsg>,
) -> Result<Ready, String> {
    let mut tunnel = None;
    if let Some(ssh) = &config.ssh {
        let known = KnownHosts {
            app_file: config_dir().join("known_hosts"),
            user_file: std::env::var_os("HOME")
                .map_or_else(PathBuf::new, PathBuf::from)
                .join(".ssh/known_hosts"),
        };
        let (host, out, tx) = (ssh.host.clone(), out.clone(), tx.clone());
        let ask: HostKeyAsk = Box::new(move |fingerprint| {
            Box::pin(async move {
                let (d_tx, d_rx) = oneshot::channel();
                // Registered before the prompt, so the decision cannot overtake it.
                if tx.send(WsMsg::AskHostKey(d_tx)).is_err() {
                    return false;
                }
                out.emit(Event::HostKeyPrompt {
                    ws,
                    host,
                    fingerprint,
                });
                d_rx.await.unwrap_or(false)
            })
        });
        tunnel = Some(
            open_tunnel(
                ssh,
                secrets.ssh,
                (config.host.clone(), config.port),
                &known,
                ask,
            )
            .await
            .map_err(|e| e.to_string())?,
        );
    }
    let target = ConnectTarget {
        host: config.host.clone(),
        port: config.port,
        connect_addr: tunnel.as_ref().map(|t| t.local_addr),
        user: config.user.clone(),
        password: secrets.db,
        database: config.database.clone(),
        tls: config.tls.clone(),
        connect_timeout: CONNECT_TIMEOUT,
    };
    let session = driver
        .connect(&target, SessionPurpose::Metadata)
        .await
        .map_err(|e| e.message)?;
    let shared = Arc::new(Shared {
        driver,
        target,
        config,
    });
    Ok(Ready {
        tunnel,
        session,
        shared,
    })
}
