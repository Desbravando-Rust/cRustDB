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

pub use dialect::MysqlDialect;
pub use introspect::MysqlIntrospector;
pub use session::MysqlSession;

#[derive(Debug, Clone, Copy, Default)]
pub struct MysqlDriver;

#[async_trait]
impl Driver for MysqlDriver {
    fn kind(&self) -> DriverKind {
        DriverKind::Mysql
    }
    fn dialect(&self) -> &dyn Dialect {
        &MysqlDialect
    }
    fn introspector(&self) -> &dyn Introspector {
        &MysqlIntrospector
    }
    async fn connect(
        &self,
        t: &ConnectTarget,
        p: SessionPurpose,
    ) -> Result<Box<dyn Session>, DriverError> {
        Ok(Box::new(connect::connect(t, p).await?))
    }
}
