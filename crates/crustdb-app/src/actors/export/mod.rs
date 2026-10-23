//! Export: the sink shared by table and result exports, and the table export task.
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use crustdb_core::dialect::build_table_query;
use crustdb_core::driver::{Session, SessionPurpose};
use crustdb_core::error::{DriverError, ErrorKind};
use crustdb_core::model::{Cell, ColumnMeta, DriverKind, Row};
use crustdb_core::query::{QueryError, TableQuery};
use tokio::sync::mpsc::UnboundedReceiver;

use super::SharedRx;
use crate::export_fmt::{Format, Out};
use crate::runtime::{Command, Emitter, Event, RequestId, TabId};

#[cfg(test)]
mod tests;

/// Rows fetched per round trip; progress is reported once per batch.
pub(super) const BATCH: usize = 5_000;

#[derive(Debug, thiserror::Error)]
pub(super) enum Failure {
    #[error("{}", .0.message)]
    Driver(#[from] DriverError),
    #[error("{0}")]
    Query(#[from] QueryError),
    #[error("write failed: {0}")]
    Io(#[from] io::Error),
    #[error("{0}")]
    Msg(&'static str),
}

/// File writes block; on the multi-thread runtime tell it so other tasks keep running.
fn blocking<T>(f: impl FnOnce() -> T) -> T {
    use tokio::runtime::{Handle, RuntimeFlavor};
    match Handle::try_current() {
        Ok(h) if h.runtime_flavor() == RuntimeFlavor::MultiThread => tokio::task::block_in_place(f),
        _ => f(),
    }
}

/// Writes to `<path>.part`; `finish` renames it into place. Dropped any other way, the
/// partial file is removed.
pub(super) struct Sink {
    out: Option<Out<File>>,
    part: PathBuf,
    path: PathBuf,
    pub(super) rows: u64,
    truncated: u64,
}

impl Drop for Sink {
    fn drop(&mut self) {
        if self.out.take().is_some() {
            let _ = fs::remove_file(&self.part);
        }
    }
}

impl Sink {
    /// The format follows `path`'s extension; `table` (quoted) is the SQL INSERT target.
    pub(super) fn create(path: &Path, table: String, kind: DriverKind) -> io::Result<Self> {
        let mut part = path.as_os_str().to_owned();
        part.push(".part");
        let part = PathBuf::from(part);
        Ok(Self {
            out: Some(Out::new(
                File::create(&part)?,
                Format::from_path(path),
                table,
                kind,
            )),
            part,
            path: path.to_path_buf(),
            rows: 0,
            truncated: 0,
        })
    }

    fn out(&mut self) -> io::Result<&mut Out<File>> {
        self.out
            .as_mut()
            .ok_or_else(|| io::Error::other("export already finished"))
    }

    pub(super) fn header(&mut self, cols: &[ColumnMeta]) -> io::Result<()> {
        self.out()?.header(cols)
    }

    pub(super) fn write(&mut self, rows: &[Row]) -> io::Result<()> {
        blocking(|| self.write_rows(rows))
    }

    fn write_rows(&mut self, rows: &[Row]) -> io::Result<()> {
        for r in rows {
            let cut = |c: &&Cell| {
                matches!(
                    c,
                    Cell::Text {
                        truncated: true,
                        ..
                    } | Cell::Bytes {
                        truncated: true,
                        ..
                    }
                )
            };
            self.truncated += r.iter().filter(cut).count() as u64;
            self.out()?.row(r)?;
        }
        self.rows += rows.len() as u64;
        Ok(())
    }

    /// Reports the cells that were cut at the memory cap before they reached the exporter.
    pub(super) fn warn_truncated(&self, out: &Emitter, tab: TabId) {
        if self.truncated > 0 {
            out.emit(Event::Notice {
                tab,
                text: format!(
                    "{} cell(s) were truncated at 1 MiB and exported truncated",
                    self.truncated
                ),
            });
        }
    }

    pub(super) fn finish(mut self) -> io::Result<u64> {
        blocking(|| {
            self.out.take().map_or(Ok(()), Out::finish)?;
            fs::rename(&self.part, &self.path)
        })
        .inspect_err(|_| {
            let _ = fs::remove_file(&self.part);
        })?;
        Ok(self.rows)
    }
}

/// Reads the active result to the end, then finishes it.
pub(super) async fn drain(
    s: &mut dyn Session,
    sink: &mut Sink,
    req: RequestId,
    out: &Emitter,
) -> Result<(), Failure> {
    loop {
        let f = s.fetch(BATCH).await?;
        sink.write(&f.rows)?;
        out.emit(Event::ExportProgress {
            req,
            rows: sink.rows,
        });
        if f.exhausted {
            s.finish().await?;
            return Ok(());
        }
    }
}

pub(super) struct TableJob {
    pub(super) tab: TabId,
    pub(super) req: RequestId,
    pub(super) query: TableQuery,
    pub(super) path: PathBuf,
    pub(super) shared: SharedRx,
    pub(super) out: Emitter,
}

/// One table export on its own `Export` session; `CancelExport` (or a dropped mailbox) cancels it.
pub(super) async fn table(job: TableJob, mut rx: UnboundedReceiver<Command>) {
    let (req, out) = (job.req, job.out.clone());
    let result = run(job, &mut rx).await.map_err(|e| e.to_string());
    out.emit(Event::ExportDone { req, result });
}

async fn run(job: TableJob, rx: &mut UnboundedReceiver<Command>) -> Result<u64, Failure> {
    let shared = job.shared.borrow().clone().ok_or_else(|| DriverError {
        kind: ErrorKind::Connect,
        message: "workspace is not connected".into(),
    })?;
    let mut target = shared.target.clone();
    target.database = Some(job.query.object.database.clone());
    let mut s = shared
        .driver
        .connect(&target, SessionPurpose::Export)
        .await?;
    let cancel = s.cancel_handle();
    // The whole table with the tab's filters and sorts: no LIMIT/OFFSET.
    let q = TableQuery {
        limit: None,
        offset: 0,
        ..job.query.clone()
    };
    let res = {
        let work = stream(s.as_mut(), &shared, &q, &job.path, &job);
        tokio::pin!(work);
        let mut open = true;
        loop {
            tokio::select! {
                r = &mut work => break r,
                m = rx.recv(), if open => match m {
                    Some(Command::CancelExport { .. }) | None => {
                        open = false;
                        let _ = cancel.cancel().await;
                    }
                    Some(_) => {}
                },
            }
        }
    };
    s.close().await;
    res
}

async fn stream(
    s: &mut dyn Session,
    shared: &super::Shared,
    q: &TableQuery,
    path: &Path,
    job: &TableJob,
) -> Result<u64, Failure> {
    let built = build_table_query(shared.driver.dialect(), q)?;
    let header = s.start_params(&built.sql, &built.params).await?;
    let d = shared.driver.dialect();
    let mut sink = Sink::create(path, d.qualified_name(&q.object), d.kind())?;
    sink.header(&header.columns)?;
    drain(s, &mut sink, job.req, &job.out).await?;
    sink.warn_truncated(&job.out, job.tab);
    Ok(sink.finish()?)
}
