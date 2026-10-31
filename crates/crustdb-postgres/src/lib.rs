mod client_auth;
mod connect;
mod dialect;
mod exec;
mod introspect;
mod session;
mod types;

use async_trait::async_trait;
use crustdb_core::dialect::Dialect;
use crustdb_core::driver::{ConnectTarget, Driver, Introspector, Session, SessionPurpose};
use crustdb_core::error::DriverError;
use crustdb_core::model::DriverKind;

pub use dialect::PgDialect;
pub use introspect::PgIntrospector;
pub use session::PgSession;

#[derive(Debug, Clone, Copy, Default)]
pub struct PostgresDriver;

#[async_trait]
impl Driver for PostgresDriver {
    fn kind(&self) -> DriverKind {
        DriverKind::Postgres
    }
    fn dialect(&self) -> &dyn Dialect {
        &PgDialect
    }
    fn introspector(&self) -> &dyn Introspector {
        &PgIntrospector
    }
    async fn connect(
        &self,
        t: &ConnectTarget,
        p: SessionPurpose,
    ) -> Result<Box<dyn Session>, DriverError> {
        Ok(Box::new(connect::connect(t, p).await?))
    }
}
