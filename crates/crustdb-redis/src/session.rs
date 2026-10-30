//! A live Redis connection owned by the application.

use crustdb_core::driver::SessionPurpose;
use crustdb_core::model::ServerVersion;
use redis::aio::MultiplexedConnection;
use tokio::task::JoinHandle;

pub(crate) struct RedisSession {
    pub(crate) conn: MultiplexedConnection,
    /// The connection's I/O loop. Dropping the connection alone does not close the socket while
    /// a command is in flight, so the loop is aborted explicitly.
    driver: JoinHandle<()>,
    pub(crate) purpose: SessionPurpose,
    pub(crate) version: ServerVersion,
    pub(crate) db: u32,
}

impl RedisSession {
    pub(crate) fn new(
        conn: MultiplexedConnection,
        driver: JoinHandle<()>,
        purpose: SessionPurpose,
        db: u32,
    ) -> Self {
        Self {
            conn,
            driver,
            purpose,
            version: crate::decode::parse_info_version(""),
            db,
        }
    }
}

impl Drop for RedisSession {
    fn drop(&mut self) {
        self.driver.abort();
    }
}
