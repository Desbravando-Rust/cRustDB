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

#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct DriverError {
    pub kind: ErrorKind,
    pub message: String,
}
