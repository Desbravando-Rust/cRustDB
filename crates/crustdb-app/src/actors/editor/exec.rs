//! Statement execution, paging and transaction buttons of the editor actor.
use super::*;

/// Drives `job` while still reading the mailbox: `CancelTab` cancels the running statement,
/// anything else waits in the backlog.
pub(super) async fn cancellable<T>(
    inbox: &mut Inbox<'_>,
    cancel: Box<dyn CancelHandle>,
    job: impl Future<Output = T>,
) -> T {
    tokio::pin!(job);
    let mut open = true;
    loop {
        tokio::select! {
            r = &mut job => return r,
            m = inbox.rx.recv(), if open => match m {
                None => {
                    open = false;
                    let _ = cancel.cancel().await;
                }
                Some(Command::CancelTab { .. }) => {
                    let _ = cancel.cancel().await;
                }
                Some(Command::CancelExport { req }) if inbox.exporting == Some(req) => {
                    let _ = cancel.cancel().await;
                }
                Some(other) => inbox.backlog.push_back(other),
            },
        }
    }
}

/// Starts `sql` and reads it. Reads stop at `limit` rows (left paused if `keep_open`);
/// everything else is drained and only the first `limit` rows are kept (D29).
async fn run_stmt(
    s: &mut dyn Session,
    sql: &str,
    mode: ExecMode,
    limit: usize,
    keep_open: bool,
) -> Result<Ran, DriverError> {
    let header = s.start(sql, mode).await?;
    let mut rows: Vec<Row> = Vec::new();
    let mut exhausted = false;
    let mut dropped = false;
    loop {
        let want = match mode {
            ExecMode::Read => limit - rows.len(),
            ExecMode::Other => ROWS_BATCH,
        };
        if want == 0 {
            break;
        }
        let f = s.fetch(want).await?;
        let room = limit - rows.len();
        dropped |= f.rows.len() > room;
        rows.extend(f.rows.into_iter().take(room));
        if f.exhausted {
            exhausted = true;
            break;
        }
    }
    let whole = exhausted;
    if !exhausted && keep_open {
        return Ok(Ran {
            header,
            rows,
            completion: None,
            exhausted: false,
            complete: false,
        });
    }
    // Always finished, even when exhausted: PG keeps an implicit transaction and MySQL reports
    // errors of later result sets here.
    let completion = s.finish().await?;
    Ok(Ran {
        header,
        rows,
        completion: Some(completion),
        exhausted: true,
        // Finished is not the same as whole: `limit` may have cut the read short.
        complete: whole && !dropped,
    })
}

async fn fetch_more(s: &mut dyn Session, limit: usize) -> Result<(Vec<Row>, bool), DriverError> {
    let mut rows: Vec<Row> = Vec::new();
    while rows.len() < limit {
        let f = s.fetch(limit - rows.len()).await?;
        rows.extend(f.rows);
        if f.exhausted {
            return Ok((rows, true));
        }
    }
    Ok((rows, false))
}

/// MySQL parses before the implicit commit, so a syntax error leaves the transaction open.
fn parse_error(e: &DriverError) -> bool {
    matches!(&e.kind, ErrorKind::Query { code: Some(c), .. } if c == "1064" || c == "1149")
}

impl EditorActor {
    fn db(&self) -> Option<String> {
        self.database
            .clone()
            .or_else(|| self.shared.borrow().as_ref()?.target.database.clone())
    }

    fn history(
        &self,
        sql: &str,
        took: Duration,
        rows: Option<u64>,
        error: Option<String>,
    ) -> HistoryEntry {
        HistoryEntry {
            ts: now_iso8601(),
            database: self.db(),
            sql: sql.to_string(),
            duration_ms: u64::try_from(took.as_millis()).unwrap_or(u64::MAX),
            rows,
            ok: error.is_none(),
            error,
        }
    }

    pub(super) async fn execute(
        &mut self,
        inbox: &mut Inbox<'_>,
        req: RequestId,
        stmts: Vec<String>,
    ) {
        if stmts.is_empty() {
            return;
        }
        self.kept = None;
        let prepared = match self.finish_paused().await {
            Ok(()) => self.ensure_session().await,
            Err(e) => Err(e),
        };
        let shared = match prepared {
            Ok(s) => s,
            Err(e) => {
                // Nothing was sent: mark it, and only report a lost session if one existed.
                let history = self.history(
                    &stmts[0],
                    Duration::ZERO,
                    None,
                    Some(format!("not sent: {}", e.message)),
                );
                if self.session.is_some() {
                    self.failed(&e);
                }
                self.out.emit(Event::ExecError {
                    req,
                    index: 0,
                    error: e,
                    history,
                });
                return;
            }
        };
        let limit = shared.config.editor_row_limit as usize;
        let last = stmts.len() - 1;
        for (i, sql) in stmts.iter().enumerate() {
            if !self
                .run_one(inbox, &shared, (req, i), sql, limit, i == last)
                .await
            {
                break;
            }
        }
    }

    /// Returns false when the batch must stop.
    async fn run_one(
        &mut self,
        inbox: &mut Inbox<'_>,
        shared: &Shared,
        (req, index): (RequestId, usize),
        sql: &str,
        limit: usize,
        last: bool,
    ) -> bool {
        let kind = shared.driver.kind();
        let c = classify(sql, kind);
        let Some(s) = self.session.as_mut() else {
            return false;
        };
        let started = Instant::now();
        let cancel = s.cancel_handle();
        let res = cancellable(
            inbox,
            cancel,
            run_stmt(s.as_mut(), sql, c.exec_mode(), limit, last),
        )
        .await;
        let took = started.elapsed();
        let r = match res {
            Ok(r) => r,
            Err(e) => {
                self.exec_failed(req, index, sql, &c, e, took).await;
                return false;
            }
        };
        let loaded = r.rows.len() as u64;
        let affected = r.completion.as_ref().and_then(|c| c.affected_rows);
        let has_result = !r.header.columns.is_empty() || !r.rows.is_empty();
        self.kept = has_result.then(|| Kept {
            header: r.header.clone(),
            rows: r.rows.clone(),
            complete: r.complete,
        });
        if has_result {
            self.out.emit(Event::ResultStarted {
                req,
                header: r.header,
            });
            for e in rows_events(req, r.rows, r.exhausted) {
                self.out.emit(e);
            }
        }
        if r.completion.is_none() {
            self.paused = Some(Paused {
                req,
                since: tokio::time::Instant::now(),
            });
        }
        let history = self.history(sql, took, affected.or(Some(loaded)), None);
        self.out.emit(Event::StatementDone {
            req,
            index,
            completion: r.completion.unwrap_or(Completion {
                affected_rows: None,
            }),
            duration: took,
            history,
        });
        self.after_success(&c, kind);
        true
    }

    fn after_success(&mut self, c: &Classified, kind: DriverKind) {
        match c.class {
            StatementClass::Tx(TxOp::Begin) => self.set_tx(TxState::Open),
            StatementClass::Tx(TxOp::Commit | TxOp::Rollback) => self.set_tx(TxState::Idle),
            // Only ROLLBACK TO can succeed in an aborted PG transaction.
            StatementClass::Tx(TxOp::Savepoint) if self.tx == TxState::Aborted => {
                self.set_tx(TxState::Open);
            }
            _ if kind == DriverKind::Mysql && c.is_ddl && self.tx == TxState::Open => {
                self.set_tx(TxState::Idle);
                self.notice("implicit commit: DDL ended the open transaction".into());
            }
            _ => {}
        }
    }

    async fn exec_failed(
        &mut self,
        req: RequestId,
        index: usize,
        sql: &str,
        c: &Classified,
        e: DriverError,
        took: Duration,
    ) {
        let history = self.history(sql, took, None, Some(e.message.clone()));
        let pg = self.kind() == DriverKind::Postgres;
        let alive = e.kind != ErrorKind::Connect;
        let ends_tx = matches!(c.class, StatementClass::Tx(TxOp::Commit | TxOp::Rollback));
        if alive {
            // Best effort: release a result the failed statement left open.
            if let Some(s) = self.session.as_mut().filter(|s| s.has_active_result()) {
                let _ = s.finish().await;
            }
            self.paused = None;
            if ends_tx {
                self.set_tx(TxState::Idle); // a failed COMMIT/ROLLBACK leaves no transaction
            } else if pg && self.tx == TxState::Open && !matches!(c.class, StatementClass::Tx(_)) {
                self.set_tx(TxState::Aborted);
            } else if !pg && c.is_ddl && self.tx == TxState::Open && !parse_error(&e) {
                self.set_tx(TxState::Idle);
                self.notice(
                    "implicit commit (statement failed): DDL ended the open transaction".into(),
                );
            }
        }
        let connect_lost = !alive;
        self.out.emit(Event::ExecError {
            req,
            index,
            error: e,
            history,
        });
        if connect_lost {
            self.lost();
        }
    }

    pub(super) async fn load_more(&mut self, inbox: &mut Inbox<'_>) {
        let (Some(req), Some(shared)) = (
            self.paused.as_ref().map(|p| p.req),
            self.shared.borrow().clone(),
        ) else {
            return self.notice("no paused result to load more from".into());
        };
        let Some(s) = self.session.as_mut() else {
            return;
        };
        let limit = shared.config.editor_row_limit as usize;
        let cancel = s.cancel_handle();
        let res = cancellable(inbox, cancel, async {
            let (rows, exhausted) = fetch_more(s.as_mut(), limit).await?;
            if exhausted {
                s.finish().await?;
            }
            Ok::<_, DriverError>((rows, exhausted))
        })
        .await;
        match res {
            Ok((rows, exhausted)) => {
                if let Some(k) = self.kept.as_mut() {
                    k.rows.extend(rows.iter().cloned());
                    k.complete = exhausted;
                }
                for e in rows_events(req, rows, exhausted) {
                    self.out.emit(e);
                }
                if exhausted {
                    self.paused = None;
                } else if let Some(p) = self.paused.as_mut() {
                    p.since = tokio::time::Instant::now();
                }
            }
            Err(e) => {
                self.notice(e.message.clone());
                self.cursor_failed(&e).await;
            }
        }
    }

    /// A fetch on the paused cursor failed (or was cancelled).
    pub(super) async fn cursor_failed(&mut self, e: &DriverError) {
        if e.kind == ErrorKind::Connect {
            self.lost();
        } else {
            self.abandon_cursor().await;
            if self.kind() == DriverKind::Postgres && self.tx == TxState::Open {
                self.set_tx(TxState::Aborted);
            }
        }
    }

    /// Best effort: release the cursor and forget it.
    pub(super) async fn abandon_cursor(&mut self) {
        self.paused = None;
        if let Some(k) = self.kept.as_mut() {
            k.complete = false;
        }
        if let Some(s) = self.session.as_mut() {
            let _ = s.finish().await;
        }
    }

    /// Runs a toolbar transaction button. Returns whether it succeeded.
    pub(super) async fn tx_op(&mut self, inbox: &mut Inbox<'_>, op: TxOp) -> bool {
        let sql = match op {
            TxOp::Begin => "BEGIN",
            TxOp::Commit => "COMMIT",
            TxOp::Rollback => "ROLLBACK",
            TxOp::Savepoint => {
                self.notice("savepoint is not a toolbar action".into());
                return false;
            }
        };
        if self.session.is_none() && op != TxOp::Begin {
            self.set_tx(TxState::Idle); // nothing is open without a session
            return true;
        }
        let prepared = match self.finish_paused().await {
            Ok(()) => self.ensure_session().await,
            Err(e) => Err(e),
        };
        if let Err(e) = prepared {
            self.notice(e.message.clone());
            if self.session.is_some() {
                self.failed(&e);
            }
            return false;
        }
        let Some(s) = self.session.as_mut() else {
            return false;
        };
        let cancel = s.cancel_handle();
        match cancellable(
            inbox,
            cancel,
            run_stmt(s.as_mut(), sql, ExecMode::Other, 0, false),
        )
        .await
        {
            Ok(_) => {
                self.set_tx(if op == TxOp::Begin {
                    TxState::Open
                } else {
                    TxState::Idle
                });
                true
            }
            Err(e) => {
                self.notice(e.message.clone());
                self.failed(&e);
                if e.kind != ErrorKind::Connect && matches!(op, TxOp::Commit | TxOp::Rollback) {
                    self.set_tx(TxState::Idle); // a failed COMMIT/ROLLBACK leaves no transaction
                }
                false
            }
        }
    }

    /// Returns true when the tab may be closed.
    pub(super) async fn close(&mut self, inbox: &mut Inbox<'_>, resolve: Option<TxOp>) -> bool {
        if self.tx == TxState::Idle {
            return true;
        }
        match resolve {
            Some(op @ (TxOp::Commit | TxOp::Rollback)) => {
                self.tx_op(inbox, op).await && self.tx == TxState::Idle
            }
            Some(_) => {
                self.notice("close needs Commit or Rollback".into());
                false
            }
            None => {
                self.notice("transaction is open: commit or roll back before closing".into());
                false
            }
        }
    }

    pub(super) async fn use_database(&mut self, database: String) {
        self.pending = None; // evaluated against the old database
        let pg = self.kind() == DriverKind::Postgres;
        if pg && self.tx != TxState::Idle {
            return self.notice("cannot change database with an open transaction".into());
        }
        if let Err(e) = self.finish_paused().await {
            self.notice(e.message.clone());
            return self.failed(&e);
        }
        if pg {
            // A PG session is bound to its database: the next run reconnects.
            if let Some(s) = self.session.take() {
                s.close().await;
            }
        } else if let Some(s) = self.session.as_mut()
            && let Err(e) = s.use_database(&database).await
        {
            self.notice(e.message.clone());
            return self.failed(&e);
        }
        self.database = Some(database);
    }
}
