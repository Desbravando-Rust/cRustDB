//! One table tab: owns a lazily opened `Data` session and serves page/structure/DDL requests.
//! Cell edits run on a second, also lazy, `Editor` session (the only writable purpose).
use std::collections::VecDeque;
use std::sync::Arc;

use crustdb_core::dialect::{BuiltQuery, build_table_query};
use crustdb_core::driver::{ResultHeader, Session, SessionPurpose};
use crustdb_core::error::{DriverError, ErrorKind};
use crustdb_core::model::{ObjectRef, Row};
use crustdb_core::query::TableQuery;
use tokio::sync::Mutex;
use tokio::sync::mpsc::UnboundedReceiver;

use super::{Shared, SharedRx};
use crate::runtime::{Command, Emitter, Event, RequestId, TabId};

struct Page {
    header: ResultHeader,
    rows: Vec<Row>,
    has_more: bool,
    estimate: Option<u64>,
}

/// Mailbox plus commands set aside while a page is running.
struct Inbox<'a> {
    rx: &'a mut UnboundedReceiver<Command>,
    backlog: VecDeque<Command>,
    /// Newest `FetchPage` seen; older ones are stale.
    latest: RequestId,
}

pub(crate) struct DataActor {
    tab: TabId,
    object: ObjectRef,
    shared: SharedRx,
    out: Emitter,
    session: Option<Box<dyn Session>>,
    writer: Option<Box<dyn Session>>,
}

impl DataActor {
    pub(crate) fn new(tab: TabId, object: ObjectRef, shared: SharedRx, out: Emitter) -> Self {
        Self {
            tab,
            object,
            shared,
            out,
            session: None,
            writer: None,
        }
    }

    pub(crate) async fn run(mut self, rx: Arc<Mutex<UnboundedReceiver<Command>>>) {
        let mut rx = rx.lock().await;
        let mut inbox = Inbox {
            rx: &mut rx,
            backlog: VecDeque::new(),
            latest: RequestId(0),
        };
        loop {
            let cmd = match inbox.backlog.pop_front() {
                Some(c) => c,
                None => match inbox.rx.recv().await {
                    Some(c) => c,
                    None => break,
                },
            };
            match cmd {
                Command::FetchPage { req, query, .. } => {
                    inbox.latest = req;
                    self.fetch_page(&mut inbox, req, query).await;
                }
                Command::UpdateCell { req, update, .. } => self.update_cell(req, update).await,
                Command::LoadStructure { req, .. } => self.structure(req).await,
                Command::LoadDdl { req, .. } => self.ddl(req).await,
                _ => {}
            }
        }
        for s in [self.session.take(), self.writer.take()]
            .into_iter()
            .flatten()
        {
            s.close().await;
        }
    }

    fn notice(&self, text: String) {
        self.out.emit(Event::Notice {
            tab: self.tab,
            text,
        });
    }

    fn shared(&self) -> Result<Arc<Shared>, DriverError> {
        self.shared.borrow().clone().ok_or_else(|| DriverError {
            kind: ErrorKind::Connect,
            message: "workspace is not connected".into(),
        })
    }

    async fn ensure_session(&mut self) -> Result<Arc<Shared>, DriverError> {
        let shared = self.shared()?;
        if self.session.is_none() {
            let target = shared.target_for(&self.object.database);
            self.session = Some(shared.driver.connect(&target, SessionPurpose::Data).await?);
        }
        Ok(shared)
    }

    /// A connection-level failure invalidates the session; the next request reconnects.
    fn failed(&mut self, e: &DriverError) {
        if matches!(
            e.kind,
            ErrorKind::Connect | ErrorKind::Protocol | ErrorKind::Timeout
        ) {
            self.session = None;
        }
        self.notice(e.message.clone());
    }

    async fn fetch_page(&mut self, inbox: &mut Inbox<'_>, req: RequestId, query: TableQuery) {
        let shared = match self.ensure_session().await {
            Ok(s) => s,
            Err(e) => return self.failed(&e),
        };
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let (res, cancelled) = drive(session.as_mut(), &shared, query, inbox).await;
        if req != inbox.latest {
            return; // superseded by a newer FetchPage
        }
        match res {
            Ok(p) => self.out.emit(Event::Page {
                req,
                header: p.header,
                rows: p.rows,
                has_more: p.has_more,
                estimate: p.estimate,
            }),
            Err(_) if cancelled => self.notice("cancelled".into()),
            Err(e) => self.failed(&e),
        }
    }

    /// Autocommit, one statement. `Some(0)` means the keyed row is gone (or, on MySQL, unchanged).
    async fn update_cell(&mut self, req: RequestId, q: BuiltQuery) {
        let r = async {
            if self.writer.is_none() {
                let shared = self.shared()?;
                let target = shared.target_for(&self.object.database);
                let s = shared
                    .driver
                    .connect(&target, SessionPurpose::Editor)
                    .await?;
                self.writer = Some(s);
            }
            let w = self.writer.as_mut().ok_or_else(closed)?;
            w.start_params(&q.sql, &q.params).await?;
            w.finish().await
        }
        .await;
        match r {
            Ok(c) if c.affected_rows == Some(0) => self.notice("row not found".into()),
            Ok(_) => self.out.emit(Event::CellUpdated { req }),
            Err(e) => {
                if matches!(
                    e.kind,
                    ErrorKind::Connect | ErrorKind::Protocol | ErrorKind::Timeout
                ) {
                    self.writer = None;
                }
                self.notice(e.message);
            }
        }
    }

    async fn structure(&mut self, req: RequestId) {
        let r = async {
            let shared = self.ensure_session().await?;
            let s = self.session.as_mut().ok_or_else(closed)?.as_mut();
            let intro = shared.driver.introspector();
            Ok::<_, DriverError>((
                intro.columns(s, &self.object).await?,
                intro.indexes(s, &self.object).await?,
                intro.foreign_keys(s, &self.object).await?,
            ))
        }
        .await;
        match r {
            Ok((columns, indexes, fks)) => self.out.emit(Event::Structure {
                req,
                columns,
                indexes,
                fks,
            }),
            Err(e) => self.failed(&e),
        }
    }

    async fn ddl(&mut self, req: RequestId) {
        let r = async {
            let shared = self.ensure_session().await?;
            let s = self.session.as_mut().ok_or_else(closed)?.as_mut();
            shared.driver.introspector().ddl(s, &self.object).await
        }
        .await;
        match r {
            Ok(text) => self.out.emit(Event::Ddl { req, text }),
            Err(e) => self.failed(&e),
        }
    }
}

fn closed() -> DriverError {
    DriverError {
        kind: ErrorKind::Connect,
        message: "session is closed".into(),
    }
}

/// Runs the page query while still reading the mailbox: a newer `FetchPage` or `CancelTab`
/// cancels the running statement. Returns the result and whether `CancelTab` was seen.
async fn drive(
    session: &mut dyn Session,
    shared: &Shared,
    query: TableQuery,
    inbox: &mut Inbox<'_>,
) -> (Result<Page, DriverError>, bool) {
    let cancel = session.cancel_handle();
    let job = read_page(session, shared, query);
    tokio::pin!(job);
    let (mut cancelled, mut open) = (false, true);
    let res = loop {
        tokio::select! {
            r = &mut job => break r,
            m = inbox.rx.recv(), if open => match m {
                None => {
                    open = false;
                    let _ = cancel.cancel().await;
                }
                Some(Command::FetchPage { tab, req, query }) => {
                    inbox.latest = req;
                    inbox.backlog.retain(|c| !matches!(c, Command::FetchPage { .. }));
                    inbox.backlog.push_back(Command::FetchPage { tab, req, query });
                    let _ = cancel.cancel().await;
                }
                Some(Command::CancelTab { .. }) => {
                    cancelled = true;
                    let _ = cancel.cancel().await;
                }
                Some(other) => inbox.backlog.push_back(other),
            },
        }
    };
    (res, cancelled)
}

/// Fetches `limit + 1` rows to learn `has_more`; the estimate is only meaningful unfiltered.
async fn read_page(
    s: &mut dyn Session,
    shared: &Shared,
    mut q: TableQuery,
) -> Result<Page, DriverError> {
    let limit = q.limit.unwrap_or(u64::from(shared.config.page_size));
    let want = usize::try_from(limit)
        .unwrap_or(usize::MAX)
        .saturating_add(1);
    let (object, filtered) = (q.object.clone(), !q.filters.is_empty());
    q.limit = Some(limit.saturating_add(1));
    let built = build_table_query(shared.driver.dialect(), &q).map_err(|e| DriverError {
        kind: ErrorKind::Query {
            code: None,
            position: None,
        },
        message: e.to_string(),
    })?;
    let header = s.start_params(&built.sql, &built.params).await?;
    let body = read_rows(s, want).await;
    let done = s.finish().await;
    let mut rows = body?;
    done?;
    let has_more = rows.len() >= want;
    rows.truncate(want - 1);
    let estimate = if filtered {
        None
    } else {
        shared
            .driver
            .introspector()
            .row_estimate(s, &object)
            .await
            .ok()
            .flatten()
    };
    Ok(Page {
        header,
        rows,
        has_more,
        estimate,
    })
}

async fn read_rows(s: &mut dyn Session, want: usize) -> Result<Vec<Row>, DriverError> {
    let mut rows = Vec::new();
    while rows.len() < want {
        let f = s.fetch(want - rows.len()).await?;
        rows.extend(f.rows);
        if f.exhausted {
            break;
        }
    }
    Ok(rows)
}
