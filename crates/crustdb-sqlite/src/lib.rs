mod conn_task;
mod connect;
mod dialect;
mod introspect;
mod session;
mod types;

use async_trait::async_trait;
use crustdb_core::dialect::Dialect;
use crustdb_core::driver::{ConnectTarget, Driver, Introspector, Session, SessionPurpose};
use crustdb_core::error::DriverError;
use crustdb_core::model::DriverKind;

pub use dialect::SqliteDialect;
pub use introspect::SqliteIntrospector;
pub use session::SqliteSession;

#[derive(Debug, Clone, Copy, Default)]
pub struct SqliteDriver;

#[async_trait]
impl Driver for SqliteDriver {
    fn kind(&self) -> DriverKind {
        DriverKind::Sqlite
    }
    fn dialect(&self) -> &dyn Dialect {
        &SqliteDialect
    }
    fn introspector(&self) -> &dyn Introspector {
        &SqliteIntrospector
    }
    /// The database file is `t.database`; host, port, user, password and TLS are ignored.
    async fn connect(
        &self,
        t: &ConnectTarget,
        p: SessionPurpose,
    ) -> Result<Box<dyn Session>, DriverError> {
        Ok(Box::new(connect::connect(t, p).await?))
    }
}
