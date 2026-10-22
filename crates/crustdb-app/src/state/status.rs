//! Status bar data (§4.8): fed by runtime events and the workspace, sampled at 1 Hz (D31).
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crustdb_core::driver::TlsMode;
use crustdb_core::model::ServerVersion;

use super::workspace::{AppState, Status, WorkspaceState};
use crate::metrics::ProcStats;
use crate::runtime::{Command, Event, RequestId, TabId, TxState, WorkspaceId};

pub const SAMPLE_EVERY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryTiming {
    pub first_row: Duration,
    pub total: Duration,
    pub render: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StatusModel {
    /// Name, server version and whether the metadata session is up.
    pub connection: Option<(String, ServerVersion, bool)>,
    pub tls: Option<TlsMode>,
    pub ssh: bool,
    pub tx: TxState,
    pub threshold: u64,
    pub last_query: Option<QueryTiming>,
    pub rows: Option<u64>,
    pub latency: Option<Duration>,
    pub proc: Option<ProcStats>,
    pub exports: Vec<(RequestId, u64)>,
    reconnecting: bool,
    tx_by_tab: HashMap<TabId, TxState>,
    render_from: Option<Instant>,
    /// Execute send time per request, then time-to-first-result once `ResultStarted` lands.
    started: HashMap<RequestId, Instant>,
    first_row: HashMap<RequestId, Duration>,
}

impl Default for StatusModel {
    fn default() -> Self {
        Self {
            connection: None,
            tls: None,
            ssh: false,
            tx: TxState::Idle,
            threshold: 0,
            last_query: None,
            rows: None,
            latency: None,
            proc: None,
            exports: vec![],
            reconnecting: false,
            tx_by_tab: HashMap::new(),
            render_from: None,
            started: HashMap::new(),
            first_row: HashMap::new(),
        }
    }
}

impl StatusModel {
    /// Fields that mirror the workspace; cheap enough to refresh every frame.
    pub fn sync(&mut self, w: &WorkspaceState) {
        self.connection = match &w.status {
            Status::Online(v) => Some((w.config.name.clone(), v.clone(), !self.reconnecting)),
            _ => None,
        };
        self.tls = Some(w.config.tls.mode);
        self.ssh = w.config.ssh.is_some();
        self.threshold = w.config.safety_threshold;
        self.tx = w
            .active_tab
            .and_then(|t| self.tx_by_tab.get(&t).copied())
            .unwrap_or(TxState::Idle);
    }

    pub fn on_command(&mut self, c: &Command, now: Instant) {
        if let Command::Execute { req, .. } = c {
            self.started.insert(*req, now);
        }
    }

    pub fn on_event(&mut self, e: &Event, now: Instant) {
        match e {
            Event::Connected { .. } => self.reconnecting = false,
            Event::Reconnecting { .. } => {
                self.reconnecting = true;
                self.latency = None;
            }
            Event::Ping { latency, .. } => {
                self.reconnecting = false;
                self.latency = Some(*latency);
            }
            Event::ResultStarted { req, .. } => {
                if let Some(t0) = self.started.get(req) {
                    self.first_row
                        .entry(*req)
                        .or_insert_with(|| now.saturating_duration_since(*t0));
                }
            }
            Event::StatementDone {
                req,
                completion,
                duration,
                history,
                ..
            } => {
                self.started.remove(req);
                let first_row = self.first_row.remove(req).unwrap_or(*duration);
                self.last_query = Some(QueryTiming {
                    first_row: first_row.min(*duration),
                    total: *duration,
                    render: Duration::ZERO,
                });
                self.rows = history.rows.or(completion.affected_rows);
                self.render_from = Some(now);
            }
            Event::TxChanged { tab, state } => {
                self.tx_by_tab.insert(*tab, *state);
            }
            Event::ExportProgress { req, rows } => {
                match self.exports.iter_mut().find(|(r, _)| r == req) {
                    Some(e) => e.1 = *rows,
                    None => self.exports.push((*req, *rows)),
                }
            }
            Event::ExportDone { req, .. } => self.exports.retain(|(r, _)| r != req),
            _ => {}
        }
    }

    /// Called once the frame that shows a fresh result is built.
    pub fn frame_done(&mut self, now: Instant) {
        if let (Some(from), Some(q)) = (self.render_from.take(), self.last_query.as_mut()) {
            q.render = now.saturating_duration_since(from);
        }
    }
}

/// One model per workspace plus the process sample they all share.
#[derive(Default)]
pub struct StatusBoard {
    models: HashMap<WorkspaceId, StatusModel>,
    proc: Option<ProcStats>,
    last_sample: Option<Instant>,
}

impl StatusBoard {
    /// Routes by workspace, then by owning tab; request-only events go to the active workspace
    /// (ponytail: wrong if the user switches workspace while a statement runs).
    pub fn on_event(&mut self, e: &Event, state: &AppState, now: Instant) {
        let by_tab = |t: &TabId| {
            state
                .workspaces
                .iter()
                .find(|w| {
                    w.tabs.iter().any(|x| x.id() == *t) || w.queries.iter().any(|q| q.id == *t)
                })
                .map(|w| w.id)
        };
        let ws = match e {
            Event::Connected { ws, .. } | Event::Reconnecting { ws } | Event::Ping { ws, .. } => {
                Some(*ws)
            }
            Event::TxChanged { tab, .. } => by_tab(tab),
            Event::StatementDone { .. }
            | Event::ResultStarted { .. }
            | Event::ExportProgress { .. }
            | Event::ExportDone { .. } => state.active,
            _ => None,
        };
        if let Some(ws) = ws {
            self.models.entry(ws).or_default().on_event(e, now);
        }
    }

    /// Execute goes to the active workspace's model (same caveat as `on_event`).
    pub fn on_command(&mut self, c: &Command, state: &AppState, now: Instant) {
        if let Some(ws) = state.active {
            self.models.entry(ws).or_default().on_command(c, now);
        }
    }

    /// Samples `/proc` at most once per `SAMPLE_EVERY`; returns whether it did.
    pub fn tick(&mut self, now: Instant, sample: impl FnOnce() -> Option<ProcStats>) -> bool {
        if self
            .last_sample
            .is_some_and(|t| now.duration_since(t) < SAMPLE_EVERY)
        {
            return false;
        }
        self.last_sample = Some(now);
        self.proc = sample();
        true
    }

    /// The active workspace's model, refreshed from `state`.
    pub fn current(&mut self, state: &AppState) -> Option<&StatusModel> {
        let w = state
            .workspaces
            .iter()
            .find(|w| Some(w.id) == state.active)?;
        let m = self.models.entry(w.id).or_default();
        m.sync(w);
        m.proc = self.proc;
        Some(m)
    }

    pub fn frame_done(&mut self, now: Instant) {
        self.models.values_mut().for_each(|m| m.frame_done(now));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn stats() -> Option<ProcStats> {
        Some(ProcStats {
            cpu_percent: 1.0,
            rss_bytes: 1,
        })
    }

    #[test]
    fn samples_once_per_second() {
        let mut b = StatusBoard::default();
        let t0 = Instant::now();
        let mut calls = 0;
        for ms in [0, 100, 900, 1000, 1500, 2100] {
            b.tick(t0 + Duration::from_millis(ms), || {
                calls += 1;
                stats()
            });
        }
        assert_eq!(calls, 3);
        assert_eq!(b.proc, stats());
    }

    #[test]
    fn first_row_is_measured_from_execute_to_result_started() {
        use crate::storage::history::HistoryEntry;
        use crustdb_core::driver::ResultHeader;
        use crustdb_core::model::Completion;
        let (t0, req, tab) = (Instant::now(), RequestId(7), TabId(1));
        let mut m = StatusModel::default();
        m.on_command(
            &Command::Execute {
                tab,
                req,
                evaluation: RequestId(6),
            },
            t0,
        );
        m.on_event(
            &Event::ResultStarted {
                req,
                header: ResultHeader { columns: vec![] },
            },
            t0 + Duration::from_millis(40),
        );
        m.on_event(
            &Event::StatementDone {
                req,
                index: 0,
                completion: Completion {
                    affected_rows: None,
                },
                duration: Duration::from_millis(100),
                history: HistoryEntry {
                    ts: String::new(),
                    database: None,
                    sql: String::new(),
                    duration_ms: 100,
                    rows: Some(1),
                    ok: true,
                    error: None,
                },
            },
            t0 + Duration::from_millis(100),
        );
        let q = m.last_query.unwrap();
        assert_eq!(q.first_row, Duration::from_millis(40));
        assert_eq!(q.total, Duration::from_millis(100));
    }
}
