use crate::model::{ColumnMeta, DriverKind, ObjectRef, TypeCategory};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    In,
    NotIn,
    Between,
    IsNull,
    IsNotNull,
    Contains,
    NotContains,
    StartsWith,
    EndsWith,
    Like,
    NotLike,
    ILike,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arity {
    Zero,
    One,
    Two,
    Many,
}

impl Operator {
    pub fn arity(&self) -> Arity {
        use Operator::*;
        match self {
            IsNull | IsNotNull => Arity::Zero,
            Between => Arity::Two,
            In | NotIn => Arity::Many,
            _ => Arity::One,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub column: String,
    pub op: Operator,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sort {
    pub column: String,
    pub descending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableQuery {
    pub object: ObjectRef,
    pub columns: Vec<ColumnMeta>,
    pub filters: Vec<Filter>,
    pub sorts: Vec<Sort>,
    /// `None` = no LIMIT/OFFSET (export).
    pub limit: Option<u64>,
    pub offset: u64,
    /// Keyset cursor: values of the `sorts` columns of the last row of the previous page.
    /// `Some` replaces `OFFSET` (ADR 0011).
    pub after: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QueryError {
    #[error("unknown column: {0}")]
    UnknownColumn(String),
    #[error("wrong number of values for {op:?}: {got}")]
    Arity { op: Operator, got: usize },
    #[error("operator {0:?} not allowed for this column type")]
    OperatorNotAllowed(Operator),
    #[error("table has no primary key")]
    NoPrimaryKey,
    #[error("column {0} is not editable")]
    NotEditable(String),
    #[error("keyset cursor has {got} values for {want} sort columns")]
    CursorLen { want: usize, got: usize },
}

use Operator::*;

const COMPARE: &[Operator] = &[
    Eq, Ne, Lt, Gt, Le, Ge, In, NotIn, Between, IsNull, IsNotNull,
];
const TEXT_PG: &[Operator] = &[
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    In,
    NotIn,
    Between,
    IsNull,
    IsNotNull,
    Contains,
    NotContains,
    StartsWith,
    EndsWith,
    Like,
    NotLike,
    ILike,
];
const TEXT_MYSQL: &[Operator] = &[
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    In,
    NotIn,
    Between,
    IsNull,
    IsNotNull,
    Contains,
    NotContains,
    StartsWith,
    EndsWith,
    Like,
    NotLike,
];
const BOOL: &[Operator] = &[Eq, Ne, IsNull, IsNotNull];
const BINARY: &[Operator] = &[IsNull, IsNotNull];

pub fn operators_for(cat: TypeCategory, kind: DriverKind) -> &'static [Operator] {
    use TypeCategory::*;
    match (cat, kind) {
        (Integer | Decimal | Float | Date | Time | Timestamp, _) => COMPARE,
        (Text | Uuid | Json | Other, DriverKind::Postgres) => TEXT_PG,
        // SQLite has no ILIKE either.
        (Text | Uuid | Json | Other, DriverKind::Mysql | DriverKind::Sqlite) => TEXT_MYSQL,
        // unreachable: the Capabilities gate (#166) keeps Redis out of SQL paths
        (Text | Uuid | Json | Other, DriverKind::Redis) => COMPARE,
        (Bool, _) => BOOL,
        (Binary, _) => BINARY,
    }
}

pub fn parse_in_list(s: &str) -> Vec<String> {
    // ponytail: values containing commas are not supported.
    s.split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_allows_only_null_checks() {
        // Act & Assert
        assert_eq!(
            operators_for(TypeCategory::Binary, DriverKind::Postgres),
            &[Operator::IsNull, Operator::IsNotNull]
        );
    }

    #[test]
    fn ilike_only_on_postgres() {
        // Act & Assert
        assert!(!operators_for(TypeCategory::Text, DriverKind::Mysql).contains(&Operator::ILike));
        assert!(operators_for(TypeCategory::Text, DriverKind::Postgres).contains(&Operator::ILike));
    }

    #[test]
    fn in_list_parsing() {
        // Act & Assert
        assert_eq!(parse_in_list(" 1, 2,,3 ,"), vec!["1", "2", "3"]);
    }
}
