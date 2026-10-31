//! Redis driver for crustdb.

pub mod classify;
mod commands;
mod connect;
mod decode;
mod session;
mod tls;
pub mod tokenize;

use async_trait::async_trait;
use crustdb_core::driver::{ConnectTarget, SessionPurpose};
use crustdb_core::error::DriverError;
use crustdb_core::model::DriverKind;
use crustdb_core::safety::Verdict;
use crustdb_core::store::{Refused, StoreDriver, StoreSession};

pub use classify::classify;

pub struct RedisDriver;

#[async_trait]
impl StoreDriver for RedisDriver {
    fn kind(&self) -> DriverKind {
        DriverKind::Redis
    }

    fn assess(&self, argv: &[Vec<u8>]) -> Result<(Verdict, String), Refused> {
        classify(argv)
    }

    async fn connect(
        &self,
        t: &ConnectTarget,
        p: SessionPurpose,
    ) -> Result<Box<dyn StoreSession>, DriverError> {
        Ok(Box::new(connect::connect(t, p).await?))
    }
}
