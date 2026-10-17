use async_trait::async_trait;
use crustdb_core::driver::{Introspector, Session};
use crustdb_core::error::{DriverError, ErrorKind};
use crustdb_core::model::{ColumnDetail, ForeignKeyInfo, IndexInfo, ObjectKind, ObjectRef};

use crate::connect::err;

/// Stub until the catalog queries land.
#[derive(Debug, Clone, Copy, Default)]
pub struct MysqlIntrospector;

fn todo_err<T>() -> Result<T, DriverError> {
    Err(err(
        ErrorKind::Unsupported,
        "MySQL introspection not implemented yet",
    ))
}

#[async_trait]
impl Introspector for MysqlIntrospector {
    async fn databases(&self, _: &mut dyn Session) -> Result<Vec<String>, DriverError> {
        todo_err()
    }
    async fn schemas(&self, _: &mut dyn Session, _: &str) -> Result<Vec<String>, DriverError> {
        todo_err()
    }
    async fn objects(
        &self,
        _: &mut dyn Session,
        _: &str,
        _: Option<&str>,
        _: ObjectKind,
    ) -> Result<Vec<ObjectRef>, DriverError> {
        todo_err()
    }
    async fn columns(
        &self,
        _: &mut dyn Session,
        _: &ObjectRef,
    ) -> Result<Vec<ColumnDetail>, DriverError> {
        todo_err()
    }
    async fn indexes(
        &self,
        _: &mut dyn Session,
        _: &ObjectRef,
    ) -> Result<Vec<IndexInfo>, DriverError> {
        todo_err()
    }
    async fn foreign_keys(
        &self,
        _: &mut dyn Session,
        _: &ObjectRef,
    ) -> Result<Vec<ForeignKeyInfo>, DriverError> {
        todo_err()
    }
    async fn ddl(&self, _: &mut dyn Session, _: &ObjectRef) -> Result<String, DriverError> {
        todo_err()
    }
    async fn row_estimate(
        &self,
        _: &mut dyn Session,
        _: &ObjectRef,
    ) -> Result<Option<u64>, DriverError> {
        todo_err()
    }
}
