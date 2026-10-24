//! Bridge between the egui thread and the tokio runtime: `Command` in, `Event` out.
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;

use crustdb_core::driver::Driver;
use crustdb_core::model::{DriverKind, Row};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

mod protocol;
pub use protocol::*;

/// Maximum rows per `Event::Rows`.
pub const ROWS_BATCH: usize = 500;

#[derive(Default)]
// `DriverKind` has no `Hash`; two entries at most, so a Vec lookup is enough.
pub struct DriverRegistry(Vec<Arc<dyn Driver>>);

impl DriverRegistry {
    pub fn with(mut self, driver: Arc<dyn Driver>) -> Self {
        self.0.retain(|d| d.kind() != driver.kind());
        self.0.push(driver);
        self
    }

    pub fn get(&self, kind: DriverKind) -> Option<Arc<dyn Driver>> {
        self.0.iter().find(|d| d.kind() == kind).cloned()
    }
}

/// Sends events to the UI and wakes it up.
#[derive(Clone)]
pub(crate) struct Emitter {
    pub(crate) tx: mpsc::Sender<Event>,
    pub(crate) repaint: Arc<dyn Fn() + Send + Sync>,
}

impl Emitter {
    pub(crate) fn emit(&self, e: Event) {
        // A closed receiver means the UI is gone; nothing left to notify.
        if self.tx.send(e).is_ok() {
            (self.repaint)();
        }
    }
}

/// Splits rows into `Event::Rows` of at most `ROWS_BATCH`; `exhausted` marks only the last one.
pub(crate) fn rows_events(req: RequestId, rows: Vec<Row>, exhausted: bool) -> Vec<Event> {
    if rows.is_empty() {
        return vec![Event::Rows {
            req,
            rows,
            exhausted,
        }];
    }
    let mut out = Vec::with_capacity(rows.len().div_ceil(ROWS_BATCH));
    let mut it = rows.into_iter().peekable();
    while it.peek().is_some() {
        let chunk: Vec<Row> = it.by_ref().take(ROWS_BATCH).collect();
        let last = it.peek().is_none();
        out.push(Event::Rows {
            req,
            rows: chunk,
            exhausted: exhausted && last,
        });
    }
    out
}

pub struct Runtime {
    cmd_tx: UnboundedSender<Command>,
    next_req: AtomicU64,
    thread: JoinHandle<()>,
}

impl Runtime {
    pub fn start(
        repaint: Arc<dyn Fn() + Send + Sync>,
        drivers: DriverRegistry,
    ) -> (Runtime, mpsc::Receiver<Event>) {
        let (ev_tx, ev_rx) = mpsc::channel();
        let (cmd_tx, cmd_rx) = unbounded_channel();
        let emitter = Emitter { tx: ev_tx, repaint };
        let thread = std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            else {
                return; // event channel closes; the UI sees a dead runtime
            };
            rt.block_on(route(cmd_rx, emitter, drivers));
        });
        let rt = Runtime {
            cmd_tx,
            next_req: AtomicU64::new(1),
            thread,
        };
        (rt, ev_rx)
    }

    pub fn send(&self, cmd: Command) {
        // Fails only after the runtime thread died; its channel close already tells the UI.
        let _ = self.cmd_tx.send(cmd);
    }

    pub fn next_request_id(&self) -> RequestId {
        RequestId(self.next_req.fetch_add(1, Ordering::Relaxed))
    }

    pub fn shutdown(self) {
        drop(self.cmd_tx);
        let _ = self.thread.join();
    }
}

/// Dispatches commands to actors by `ws`/`tab`. Commands without an actor
/// (editor/export, until tasks 23-24) are answered with a notice.
async fn route(mut rx: UnboundedReceiver<Command>, out: Emitter, drivers: DriverRegistry) {
    let mut actors = crate::actors::Actors::new(drivers, out.clone());
    while let Some(cmd) = rx.recv().await {
        if let Some(cmd) = actors.dispatch(cmd) {
            out.emit(no_actor(&cmd));
        }
    }
}

fn no_actor(cmd: &Command) -> Event {
    use Command::*;
    if let Tx {
        tab,
        op: TxOp::Savepoint,
    }
    | CloseTab {
        tab,
        resolve: Some(TxOp::Savepoint),
    } = cmd
    {
        return Event::Notice {
            tab: *tab,
            text: "savepoint is not a toolbar action".into(),
        };
    }
    let tab = match cmd {
        OpenTable { tab, .. }
        | FetchPage { tab, .. }
        | UpdateCell { tab, .. }
        | LoadStructure { tab, .. }
        | LoadDdl { tab, .. }
        | OpenEditor { tab, .. }
        | Evaluate { tab, .. }
        | Execute { tab, .. }
        | LoadMore { tab, .. }
        | Tx { tab, .. }
        | UseDatabase { tab, .. }
        | CancelTab { tab }
        | CloseTab { tab, .. }
        | ExportTable { tab, .. }
        | ExportResult { tab, .. } => *tab,
        // Commands without a tab (ws-level or export) are reported on tab 0.
        Connect { ws, .. } => {
            return Event::ConnectFailed {
                ws: *ws,
                error: "workspace actor not available".into(),
            };
        }
        Disconnect { .. }
        | HostKeyDecision { .. }
        | LoadChildren { .. }
        | LoadColumns { .. }
        | LoadDiagram { .. }
        | CancelExport { .. } => TabId(0),
    };
    Event::Notice {
        tab,
        text: "no actor for this command".into(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    use crustdb_core::driver::{ConnectTarget, SessionPurpose, TlsConfig, TlsMode};
    use crustdb_core::model::Cell;
    use crustdb_testkit::fake_driver::{FakeDriver, FakeScript};

    use super::*;

    fn registry() -> DriverRegistry {
        DriverRegistry::default().with(FakeDriver::new(FakeScript::default()))
    }

    #[test]
    fn request_ids_are_monotonic() {
        // Arrange
        let (rt, _rx) = Runtime::start(Arc::new(|| {}), registry());
        let a = rt.next_request_id();

        // Act & Assert
        let b = rt.next_request_id();
        assert!(b.0 > a.0);
        rt.shutdown();
    }

    #[test]
    fn events_wake_ui_via_repaint() {
        // Arrange
        let n = Arc::new(AtomicUsize::new(0));
        let n2 = n.clone();
        let (rt, rx) = Runtime::start(
            Arc::new(move || {
                n2.fetch_add(1, Ordering::SeqCst);
            }),
            registry(),
        );
        for _ in 0..3 {
            rt.send(Command::CancelTab { tab: TabId(1) });
        }

        // Act & Assert
        for _ in 0..3 {
            let ev = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(matches!(ev, Event::Notice { tab: TabId(1), .. }));
        }
        rt.shutdown(); // join: all repaints done
        assert_eq!(n.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn rows_batched_at_most_500() {
        // Arrange
        let rows: Vec<Row> = (0..1200)
            .map(|i| {
                vec![Cell::Text {
                    value: i.to_string().into(),
                    truncated: false,
                }]
                .into()
            })
            .collect();
        let driver = FakeDriver::new(FakeScript {
            rows,
            ..FakeScript::default()
        });
        let target = ConnectTarget {
            host: "h".into(),
            port: 1,
            connect_addr: None,
            user: "u".into(),
            password: None,
            database: None,
            tls: TlsConfig {
                mode: TlsMode::Disable,
                ca_path: None,
            },
            connect_timeout: Duration::from_secs(1),
        };
        let mut s = driver.connect(&target, SessionPurpose::Data).await.unwrap();
        s.start("select 1", crustdb_core::driver::ExecMode::Read)
            .await
            .unwrap();
        let f = s.fetch(2000).await.unwrap();

        // Act
        let evs = rows_events(RequestId(1), f.rows, f.exhausted);

        // Assert
        let sizes: Vec<_> = evs
            .iter()
            .map(|e| match e {
                Event::Rows {
                    rows, exhausted, ..
                } => (rows.len(), *exhausted),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(sizes, [(500, false), (500, false), (200, true)]);
    }

    #[test]
    fn savepoint_is_not_a_toolbar_action() {
        // Arrange
        let (rt, rx) = Runtime::start(Arc::new(|| {}), registry());
        rt.send(Command::Tx {
            tab: TabId(2),
            op: TxOp::Savepoint,
        });

        // Act & Assert
        let ev = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(ev, Event::Notice { tab: TabId(2), text } if text.contains("savepoint")));
        rt.shutdown();
    }

    #[test]
    fn shutdown_joins_runtime_thread() {
        // Arrange
        let (rt, rx) = Runtime::start(Arc::new(|| {}), registry());

        // Act
        rt.shutdown();

        // Assert
        // shutdown() returned, so the thread is joined and its emitter dropped.
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
    }
}
