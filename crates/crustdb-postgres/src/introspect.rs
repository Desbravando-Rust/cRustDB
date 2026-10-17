use async_trait::async_trait;
use crustdb_core::driver::{Introspector, Session};
use crustdb_core::error::DriverError;
use crustdb_core::model::{ColumnDetail, ForeignKeyInfo, IndexInfo, ObjectKind, ObjectRef};

use crate::session::unsupported;

/// Filled in by the introspection task.
pub struct PgIntrospector;

#[async_trait]
impl Introspector for PgIntrospector {
    async fn databases(&self, _: &mut dyn Session) -> Result<Vec<String>, DriverError> {
        Err(unsupported())
    }
    async fn schemas(&self, _: &mut dyn Session, _: &str) -> Result<Vec<String>, DriverError> {
        Err(unsupported())
    }
    async fn objects(
        &self,
        _: &mut dyn Session,
        _: &str,
        _: Option<&str>,
        _: ObjectKind,
    ) -> Result<Vec<ObjectRef>, DriverError> {
        Err(unsupported())
    }
    async fn columns(
        &self,
        _: &mut dyn Session,
        _: &ObjectRef,
    ) -> Result<Vec<ColumnDetail>, DriverError> {
        Err(unsupported())
    }
    async fn indexes(
        &self,
        _: &mut dyn Session,
        _: &ObjectRef,
    ) -> Result<Vec<IndexInfo>, DriverError> {
        Err(unsupported())
    }
    async fn foreign_keys(
        &self,
        _: &mut dyn Session,
        _: &ObjectRef,
    ) -> Result<Vec<ForeignKeyInfo>, DriverError> {
        Err(unsupported())
    }
    async fn ddl(&self, _: &mut dyn Session, _: &ObjectRef) -> Result<String, DriverError> {
        Err(unsupported())
    }
    async fn row_estimate(
        &self,
        _: &mut dyn Session,
        _: &ObjectRef,
    ) -> Result<Option<u64>, DriverError> {
        Err(unsupported())
    }
}
