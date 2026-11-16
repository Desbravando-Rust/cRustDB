#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    Connect,
    Auth,
    Tls,
    Timeout,
    Query {
        code: Option<String>,
        position: Option<usize>,
    },
    Cancelled,
    Unsupported,
    Protocol,
}

impl ErrorKind {
    /// The session's stream is unusable: the next request must reconnect.
    pub fn ends_session(&self) -> bool {
        matches!(self, ErrorKind::Connect | ErrorKind::Protocol)
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct DriverError {
    pub kind: ErrorKind,
    pub message: String,
}

impl DriverError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// A query error without server code or position.
    pub fn query(message: impl Into<String>) -> Self {
        Self::new(
            ErrorKind::Query {
                code: None,
                position: None,
            },
            message,
        )
    }

    /// The connect deadline passed.
    pub fn timeout() -> Self {
        Self::new(ErrorKind::Timeout, "connection timed out")
    }

    /// The introspector has no catalog query for `kind`.
    pub fn no_catalog_query(kind: crate::model::ObjectKind) -> Self {
        Self::new(
            ErrorKind::Unsupported,
            format!("no catalog query for {kind:?}"),
        )
    }
}

/// D39: a server without TLS 1.2+ gets a fixed, actionable message.
pub const NO_TLS12: &str = "server does not support TLS 1.2+; use an SSH tunnel or disable TLS";
