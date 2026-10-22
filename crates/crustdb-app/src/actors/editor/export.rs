//! D26: exports the last result from inside the editor: loaded rows, then the rest of the cursor.
use super::super::export::{BATCH, Failure, Sink, drain};
use super::exec::cancellable;
use super::*;

/// The last statement's header and loaded rows, kept so an export does not need the UI's copy.
// ponytail: a second copy of the loaded rows that grows with every Load more; upgrade is
// `Row` as `Arc<[Cell]>` in the core so both copies share the cells.
pub(super) struct Kept {
    pub(super) header: ResultHeader,
    pub(super) rows: Vec<Row>,
    /// Every row of the result is in `rows`; otherwise an export is partial (D26/D29).
    pub(super) complete: bool,
}

impl EditorActor {
    pub(super) async fn export_result(
        &mut self,
        inbox: &mut Inbox<'_>,
        req: RequestId,
        path: std::path::PathBuf,
    ) {
        let result = self
            .write_result(inbox, req, &path)
            .await
            .map_err(|e| e.to_string());
        self.out.emit(Event::ExportDone { req, result });
    }

    async fn write_result(
        &mut self,
        inbox: &mut Inbox<'_>,
        req: RequestId,
        path: &std::path::Path,
    ) -> Result<u64, Failure> {
        // A cancel that arrived while this command waited in the backlog.
        if let Some(i) = inbox
            .backlog
            .iter()
            .position(|c| matches!(c, Command::CancelExport { req: r } if *r == req))
        {
            inbox.backlog.remove(i);
            return Err(Failure::Msg("cancelled"));
        }
        let Some(mut kept) = self.kept.take() else {
            return Err(Failure::Msg("no result to export"));
        };
        let r = self.copy_result(inbox, req, path, &mut kept).await;
        // Drained rows are not in `kept`: it is whole only while the cursor was not read.
        if !matches!(r, Ok((_, true))) {
            self.kept = Some(kept);
        }
        match r {
            Ok((n, _)) => Ok(n),
            Err((e, started)) => {
                // Only a failed read says something about the cursor.
                if started {
                    match &e {
                        Failure::Driver(d) => self.cursor_failed(d).await,
                        _ => self.abandon_cursor().await,
                    }
                }
                Err(e)
            }
        }
    }

    /// Ok: rows written and whether the cursor was drained. Err: failure and whether the read of
    /// the cursor had started.
    async fn copy_result(
        &mut self,
        inbox: &mut Inbox<'_>,
        req: RequestId,
        path: &std::path::Path,
        kept: &mut Kept,
    ) -> Result<(u64, bool), (Failure, bool)> {
        let out = self.out.clone();
        let mut sink = Sink::create(path).map_err(|e| (e.into(), false))?;
        let head = (|| {
            sink.header(&kept.header.columns)?;
            for chunk in kept.rows.chunks(BATCH) {
                sink.write(chunk)?;
                out.emit(Event::ExportProgress {
                    req,
                    rows: sink.rows,
                });
            }
            Ok::<_, std::io::Error>(())
        })();
        head.map_err(|e| (e.into(), false))?;
        let mut drained = false;
        if self.paused.is_some()
            && let Some(s) = self.session.as_mut()
        {
            inbox.exporting = Some(req);
            let cancel = s.cancel_handle();
            let r = cancellable(inbox, cancel, drain(s.as_mut(), &mut sink, req, &out)).await;
            inbox.exporting = None;
            r.map_err(|e| (e, true))?;
            self.paused = None;
            drained = true;
        } else if !kept.complete {
            self.notice(format!(
                "Partial export: only the {} rows kept from the result were written",
                kept.rows.len()
            ));
        }
        sink.warn_truncated(&out, self.tab);
        let n = sink.finish().map_err(|e| (e.into(), false))?;
        Ok((n, drained))
    }
}
