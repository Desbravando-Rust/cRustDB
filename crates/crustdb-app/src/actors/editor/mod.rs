//! One query tab: owns a lazily opened `Editor` session; evaluates, runs and pages SQL.
use std::collections::VecDeque;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crustdb_core::driver::{CancelHandle, ExecMode, ResultHeader, Session, SessionPurpose};
use crustdb_core::error::{DriverError, ErrorKind};
use crustdb_core::model::{Completion, DriverKind, ObjectKind, ObjectRef, Row};
use crustdb_core::safety::{Assessment, EstimateNeed, SafetyPolicy, estimate_need};
use crustdb_core::sql::{Classified, StatementClass, classify};
use tokio::sync::Mutex;
use tokio::sync::mpsc::UnboundedReceiver;

#[cfg(test)]
mod eval_tests;
mod exec;
mod export;
#[cfg(test)]
mod fix_tests;
#[cfg(test)]
pub(super) mod tests;

use self::export::Kept;
use super::{Shared, SharedRx};
use crate::runtime::{
    Command, Emitter, Event, ROWS_BATCH, RequestId, TabId, TxOp, TxState, rows_events,
};
use crate::storage::history::{HistoryEntry, now_iso8601};

/// D37: a paused cursor idle for this long is finished.
const PAUSE_TIMEOUT: Duration = Duration::from_secs(300);
const LOST: &str = "Connection lost — open transaction was rolled back by the server";

/// The last statement's cursor, left open for `LoadMore`.
struct Paused {
    req: RequestId,
    since: tokio::time::Instant,
}

/// Result of driving one statement on the session.
struct Ran {
    header: ResultHeader,
    rows: Vec<Row>,
    /// `Some` once the result was finished; `None` while the cursor is left paused.
    completion: Option<Completion>,
    exhausted: bool,
    /// Every row of the result is in `rows` (nothing cut by `limit`, cursor read to the end).
    complete: bool,
}

struct Inbox<'a> {
    rx: &'a mut UnboundedReceiver<Command>,
    backlog: VecDeque<Command>,
    /// The export that `CancelExport` may cancel; a stale cancel is ignored.
    exporting: Option<RequestId>,
}

pub(crate) struct EditorActor {
    tab: TabId,
    shared: SharedRx,
    out: Emitter,
    database: Option<String>,
    session: Option<Box<dyn Session>>,
    tx: TxState,
    paused: Option<Paused>,
    /// The only evaluation `Execute` may run: id and the exact statements evaluated.
    pending: Option<(RequestId, Vec<String>)>,
    /// What the last statement returned, for `ExportResult`.
    kept: Option<Kept>,
}

impl EditorActor {
    pub(crate) fn new(
        tab: TabId,
        database: Option<String>,
        shared: SharedRx,
        out: Emitter,
    ) -> Self {
        Self {
            tab,
            shared,
            out,
            database,
            session: None,
            tx: TxState::Idle,
            paused: None,
            pending: None,
            kept: None,
        }
    }

    pub(crate) async fn run(mut self, rx: Arc<Mutex<UnboundedReceiver<Command>>>) {
        let mut rx = rx.lock().await;
        let mut inbox = Inbox {
            rx: &mut rx,
            backlog: VecDeque::new(),
            exporting: None,
        };
        loop {
            let cmd = if let Some(c) = inbox.backlog.pop_front() {
                c
            } else if let Some(p) = &self.paused {
                tokio::select! {
                    c = inbox.rx.recv() => match c { Some(c) => c, None => break },
                    () = tokio::time::sleep_until(p.since + PAUSE_TIMEOUT) => {
                        self.expire_cursor().await;
                        continue;
                    }
                }
            } else {
                match inbox.rx.recv().await {
                    Some(c) => c,
                    None => break,
                }
            };
            if !self.handle(&mut inbox, cmd).await {
                break;
            }
        }
        if let Some(s) = self.session.take() {
            s.close().await;
        }
    }

    /// Returns false when the tab is closed.
    async fn handle(&mut self, inbox: &mut Inbox<'_>, cmd: Command) -> bool {
        match cmd {
            Command::Evaluate {
                req, statements, ..
            } => self.evaluate(req, statements).await,
            Command::Execute {
                req, evaluation, ..
            } => match self.pending.take_if(|(id, _)| *id == evaluation) {
                Some((_, stmts)) => self.execute(inbox, req, stmts).await,
                None => {
                    self.notice("unknown or already used evaluation; nothing was executed".into())
                }
            },
            Command::LoadMore { .. } => self.load_more(inbox).await,
            Command::Tx { op, .. } => {
                self.tx_op(inbox, op).await;
            }
            Command::UseDatabase { database, .. } => self.use_database(database).await,
            Command::ExportResult { req, path, .. } => self.export_result(inbox, req, path).await,
            Command::CloseTab { resolve, .. } => return !self.close(inbox, resolve).await,
            _ => {}
        }
        true
    }

    fn notice(&self, text: String) {
        self.out.emit(Event::Notice {
            tab: self.tab,
            text,
        });
    }

    fn set_tx(&mut self, state: TxState) {
        if self.tx != state {
            self.tx = state;
            self.out.emit(Event::TxChanged {
                tab: self.tab,
                state,
            });
        }
    }

    fn kind(&self) -> DriverKind {
        self.shared
            .borrow()
            .as_ref()
            .map_or(DriverKind::Postgres, |s| s.driver.kind())
    }

    /// The session died: the server rolled back whatever was open.
    fn lost(&mut self) {
        self.session = None;
        self.pending = None;
        self.paused = None;
        self.kept = None;
        self.tx = TxState::Idle;
        self.notice(LOST.into());
        self.out.emit(Event::TxChanged {
            tab: self.tab,
            state: TxState::Idle,
        });
    }

    fn failed(&mut self, e: &DriverError) {
        if e.kind == ErrorKind::Connect {
            self.lost();
        }
    }

    async fn ensure_session(&mut self) -> Result<Arc<Shared>, DriverError> {
        let shared = self.shared.borrow().clone().ok_or_else(|| DriverError {
            kind: ErrorKind::Connect,
            message: "workspace is not connected".into(),
        })?;
        if self.session.is_none() {
            let mut target = shared.target.clone();
            if self.database.is_some() {
                target.database.clone_from(&self.database);
            }
            self.session = Some(
                shared
                    .driver
                    .connect(&target, SessionPurpose::Editor)
                    .await?,
            );
        }
        Ok(shared)
    }

    /// Finishes the paused cursor (PG: closes the implicit transaction; MySQL: drains).
    async fn finish_paused(&mut self) -> Result<(), DriverError> {
        if self.paused.take().is_some() {
            if let Some(k) = self.kept.as_mut() {
                k.complete = false;
            }
            if let Some(s) = self.session.as_mut() {
                s.finish().await?;
            }
        }
        Ok(())
    }

    async fn expire_cursor(&mut self) {
        match self.finish_paused().await {
            Ok(()) => self.notice("Paused result closed after 5 minutes idle".into()),
            Err(e) => self.failed(&e),
        }
    }

    async fn evaluate(&mut self, req: RequestId, statements: Vec<String>) {
        self.pending = None;
        let Some(shared) = self.shared.borrow().clone() else {
            return self.notice("workspace is not connected".into());
        };
        let kind = shared.driver.kind();
        let classified: Vec<Classified> = statements.iter().map(|s| classify(s, kind)).collect();
        let needs: Vec<EstimateNeed> = classified.iter().map(estimate_need).collect();
        let mut estimates = vec![None; statements.len()];
        if needs.iter().any(|n| *n != EstimateNeed::None) && self.prepare_session().await {
            // Earlier DDL (the table may not exist yet) or USE/SET (the target may change) makes
            // EXPLAIN untrustworthy for the rest of the batch; an earlier DML on the same table
            // makes only that table's estimates stale.
            // ponytail: coarse on purpose; refine to name matching if it proves too strict.
            let (mut untrusted, mut written) = (false, Vec::<String>::new());
            for (i, sql) in statements.iter().enumerate() {
                let c = &classified[i];
                let stale = c
                    .target
                    .as_ref()
                    .is_some_and(|t| written.iter().any(|w| w.eq_ignore_ascii_case(&t.name)));
                if !untrusted && !stale {
                    estimates[i] = self.estimate(&shared, needs[i], c, sql).await;
                }
                // Unknown (CALL/DO, set_config(...)) may change the session in ways we cannot see.
                untrusted |= c.is_ddl
                    || matches!(c.class, StatementClass::Session | StatementClass::Unknown);
                if let (StatementClass::Dml(_), Some(t)) = (c.class, &c.target) {
                    written.push(t.name.clone());
                }
            }
        }
        let items: Vec<Assessment> = statements
            .iter()
            .zip(classified)
            .zip(estimates)
            .enumerate()
            .map(|(index, ((sql, classified), estimate))| Assessment {
                index,
                sql: sql.clone(),
                classified,
                estimate,
            })
            .collect();
        let evaluation = SafetyPolicy {
            threshold: shared.config.safety_threshold,
        }
        .evaluate(&items);
        self.pending = Some((req, statements));
        self.out.emit(Event::Evaluated { req, evaluation });
    }

    /// Connects and finishes any paused cursor so the session can run EXPLAIN.
    async fn prepare_session(&mut self) -> bool {
        let r = match self.ensure_session().await {
            Ok(_) => self.finish_paused().await,
            Err(e) => Err(e),
        };
        match r {
            Ok(()) => true,
            Err(e) => {
                if self.session.is_some() {
                    self.failed(&e);
                }
                false
            }
        }
    }

    /// Any failure yields `None`, which the policy treats as "unknown" (D23).
    async fn estimate(
        &mut self,
        shared: &Shared,
        need: EstimateNeed,
        c: &Classified,
        sql: &str,
    ) -> Option<u64> {
        let s = self.session.as_mut()?.as_mut();
        let r = match need {
            EstimateNeed::None => return None,
            EstimateNeed::Explain => s.explain_rows(sql).await,
            EstimateNeed::TableRows => {
                let pg = shared.driver.kind() == DriverKind::Postgres;
                // The classifier keeps only the first table: a list or CASCADE (which reaches
                // other tables) leaves the total unknown. PG resolves an unqualified name by
                // search_path/temp tables, which the introspector cannot reproduce.
                let cascade = sql
                    .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
                    .any(|w| w.eq_ignore_ascii_case("cascade"));
                let t = c
                    .target
                    .as_ref()
                    .filter(|t| !sql.contains(',') && !cascade && !(pg && t.qualifier.is_none()))?;
                let o = ObjectRef {
                    database: t
                        .qualifier
                        .clone()
                        .filter(|_| !pg)
                        .or_else(|| self.database.clone())
                        .or_else(|| shared.target.database.clone())
                        .unwrap_or_default(),
                    schema: t.qualifier.clone().filter(|_| pg),
                    name: t.name.clone(),
                    kind: ObjectKind::Table,
                };
                shared.driver.introspector().row_estimate(s, &o).await
            }
        };
        match r {
            Ok(n) => n,
            Err(e) => {
                self.failed(&e);
                // A failed statement aborts an open PG transaction on the server.
                if e.kind != ErrorKind::Connect
                    && shared.driver.kind() == DriverKind::Postgres
                    && self.tx == TxState::Open
                {
                    self.set_tx(TxState::Aborted);
                    self.notice("transaction aborted by a failed EXPLAIN".into());
                }
                None
            }
        }
    }
}
