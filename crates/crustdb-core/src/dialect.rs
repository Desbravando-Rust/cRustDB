use crate::model::{ColumnMeta, DriverKind, ObjectRef};

pub trait Dialect: Send + Sync {
    fn kind(&self) -> DriverKind;
    fn quote_ident(&self, s: &str) -> String;
    fn qualified_name(&self, o: &ObjectRef) -> String;
    fn param(&self, index: usize, column: &ColumnMeta) -> String;
    fn select_expr(&self, column: &ColumnMeta) -> String;
    fn ci_like(&self, column: &ColumnMeta, param: &str) -> String;
}
