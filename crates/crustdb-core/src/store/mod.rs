pub mod keyspace;
pub mod value;

use std::time::Duration;

use async_trait::async_trait;

use crate::driver::{ConnectTarget, SessionPurpose};
use crate::error::{DriverError, ErrorKind};
use crate::model::{DriverKind, ServerVersion};
use crate::safety::Verdict;

use keyspace::EntryKey;
use value::ValueBody;

/// A command the driver's safety policy refuses to run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("command refused: {reason}")]
pub struct Refused {
    pub reason: String,
}

impl From<Refused> for DriverError {
    fn from(r: Refused) -> Self {
        DriverError {
            kind: ErrorKind::Unsupported,
            message: r.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueKind {
    Text,
    Map,
    List,
    Set,
    SortedSet,
    Stream,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanRequest {
    pub cursor: Option<String>,
    pub pattern: Option<String>,
    pub count: u32,
}

/// `ttl: None` means the key does not expire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEntry {
    pub key: EntryKey,
    pub kind: ValueKind,
    pub ttl: Option<Duration>,
}

/// `next: None` means the scan is complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanPage {
    pub entries: Vec<KeyEntry>,
    pub next: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntryView {
    pub ttl: Option<Duration>,
    pub size_bytes: Option<u64>,
    pub body: ValueBody,
    pub truncated: bool,
}

/// Raw command reply, protocol-neutral.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Nil,
    Int(i64),
    Double(f64),
    Bool(bool),
    Bulk(Vec<u8>),
    Status(String),
    Array(Vec<Reply>),
    Map(Vec<(Reply, Reply)>),
}

#[async_trait]
pub trait StoreDriver: Send + Sync {
    fn kind(&self) -> DriverKind;
    /// Classifies a command; `Err` means it must not run at all.
    fn assess(&self, argv: &[Vec<u8>]) -> Result<(Verdict, String), Refused>;
    async fn connect(
        &self,
        t: &ConnectTarget,
        p: SessionPurpose,
    ) -> Result<Box<dyn StoreSession>, DriverError>;
}

/// Bound to `ConnectTarget.database`; changing namespace means reconnecting.
/// Cancelling a command means dropping the session.
#[async_trait]
pub trait StoreSession: Send {
    fn server_version(&self) -> &ServerVersion;
    async fn ping(&mut self) -> Result<Duration, DriverError>;
    async fn namespaces(&mut self) -> Result<Vec<String>, DriverError>;
    async fn scan(&mut self, req: &ScanRequest) -> Result<ScanPage, DriverError>;
    /// Reads at most `limit` elements; `EntryView.truncated` reports the cut.
    async fn read(&mut self, key: &EntryKey, limit: usize) -> Result<EntryView, DriverError>;
    /// Runs an arbitrary command; refusals surface as `Refused` via `From`.
    async fn run(&mut self, argv: &[Vec<u8>]) -> Result<Reply, DriverError>;
    async fn close(self: Box<Self>);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn refused_converts_to_unsupported_driver_error() {
        // Arrange
        let refused = Refused {
            reason: "FLUSHALL".into(),
        };

        // Act
        let err = DriverError::from(refused);

        // Assert
        assert_eq!(err.kind, ErrorKind::Unsupported);
        assert_eq!(err.message, "command refused: FLUSHALL");
    }
}
