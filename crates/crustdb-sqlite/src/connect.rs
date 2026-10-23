use crustdb_core::driver::{ConnectTarget, SessionPurpose};
use crustdb_core::error::{DriverError, ErrorKind};
use rusqlite::{ErrorCode, OpenFlags};
use tokio::sync::{mpsc, oneshot};

use crate::conn_task;
use crate::session::SqliteSession;

pub(crate) fn err(kind: ErrorKind, message: impl Into<String>) -> DriverError {
    DriverError {
        kind,
        message: message.into(),
    }
}

pub(crate) fn map_err(e: rusqlite::Error) -> DriverError {
    let message = e.to_string();
    let kind = match &e {
        rusqlite::Error::SqliteFailure(f, _) => match f.code {
            ErrorCode::OperationInterrupted => ErrorKind::Cancelled,
            ErrorCode::CannotOpen | ErrorCode::NotADatabase => ErrorKind::Connect,
            _ => ErrorKind::Query {
                code: Some(f.extended_code.to_string()),
                position: None,
            },
        },
        _ => ErrorKind::Query {
            code: None,
            position: None,
        },
    };
    err(kind, message)
}

/// Opens an existing file only (never creates one). Non-editor sessions are read-only.
pub(crate) async fn connect(
    t: &ConnectTarget,
    purpose: SessionPurpose,
) -> Result<SqliteSession, DriverError> {
    let path = t
        .database
        .clone()
        .filter(|d| !d.is_empty())
        .ok_or_else(|| err(ErrorKind::Connect, "no SQLite database file set"))?;
    let mode = if purpose == SessionPurpose::Editor {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let flags = mode | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let (tx, rx) = mpsc::channel(1);
    let (ready, ready_rx) = oneshot::channel();
    let thread = std::thread::Builder::new()
        .name("crustdb-sqlite".into())
        .spawn(move || conn_task::run(path, flags, ready, rx))
        .map_err(|e| err(ErrorKind::Connect, e.to_string()))?;
    let (interrupt, raw_version) = ready_rx
        .await
        .map_err(|_| err(ErrorKind::Connect, "connection thread ended"))??;
    Ok(SqliteSession::new(tx, thread, interrupt, raw_version))
}
