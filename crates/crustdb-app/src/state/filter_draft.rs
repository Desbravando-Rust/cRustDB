use crustdb_core::model::{ColumnMeta, DriverKind};
use crustdb_core::query::{Arity, Filter, Operator, operators_for, parse_in_list};

pub fn label(op: Operator) -> &'static str {
    use Operator::*;
    match op {
        Eq => "=",
        Ne => "!=",
        Lt => "<",
        Gt => ">",
        Le => "<=",
        Ge => ">=",
        In => "IN",
        NotIn => "NOT IN",
        Between => "BETWEEN",
        IsNull => "IS NULL",
        IsNotNull => "IS NOT NULL",
        Contains => "contains",
        NotContains => "not contains",
        StartsWith => "starts with",
        EndsWith => "ends with",
        Like => "LIKE",
        NotLike => "NOT LIKE",
        ILike => "ILIKE",
    }
}

/// A filter row being edited in the bar; becomes a `Filter` on apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterDraft {
    pub column: String,
    pub op: Operator,
    pub input: String,
}

impl FilterDraft {
    pub fn new(columns: &[ColumnMeta]) -> Self {
        Self {
            column: columns.first().map(|c| c.name.clone()).unwrap_or_default(),
            op: Operator::Eq,
            input: String::new(),
        }
    }

    /// Operators offered for the draft's column (by catalog type category).
    pub fn operators(&self, columns: &[ColumnMeta], kind: DriverKind) -> &'static [Operator] {
        columns
            .iter()
            .find(|c| c.name == self.column)
            .map_or(&[], |c| operators_for(c.category, kind))
    }

    /// Switches column, keeping the operator only if the new column allows it.
    pub fn set_column(&mut self, columns: &[ColumnMeta], kind: DriverKind, name: &str) {
        self.column = name.into();
        let ops = self.operators(columns, kind);
        if !ops.contains(&self.op) {
            self.op = ops.first().copied().unwrap_or(self.op);
        }
    }

    pub fn to_filter(&self, columns: &[ColumnMeta], kind: DriverKind) -> Result<Filter, String> {
        if !columns.iter().any(|c| c.name == self.column) {
            return Err(format!("unknown column: {}", self.column));
        }
        if !self.operators(columns, kind).contains(&self.op) {
            return Err(format!(
                "{} not allowed for {}",
                label(self.op),
                self.column
            ));
        }
        let input = self.input.trim();
        let values = match self.op.arity() {
            Arity::Zero => vec![],
            Arity::One if input.is_empty() => return Err("value required".into()),
            Arity::One => vec![input.to_string()],
            Arity::Two => {
                let v: Vec<_> = input.split(',').map(|s| s.trim().to_string()).collect();
                if v.len() != 2 || v.iter().any(String::is_empty) {
                    return Err("BETWEEN needs two values separated by a comma".into());
                }
                v
            }
            Arity::Many => {
                let v = parse_in_list(input);
                if v.is_empty() {
                    return Err("at least one value required".into());
                }
                v
            }
        };
        Ok(Filter {
            column: self.column.clone(),
            op: self.op,
            values,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crustdb_core::model::TypeCategory;

    use super::*;

    fn col(name: &str, category: TypeCategory) -> ColumnMeta {
        ColumnMeta {
            name: name.into(),
            type_name: "t".into(),
            category,
            nullable: None,
            is_primary_key: false,
        }
    }

    fn draft(column: &str, op: Operator, input: &str) -> FilterDraft {
        FilterDraft {
            column: column.into(),
            op,
            input: input.into(),
        }
    }

    #[test]
    fn operator_list_follows_column_category() {
        let cols = [
            col("n", TypeCategory::Integer),
            col("b", TypeCategory::Binary),
        ];
        let d = |c| draft(c, Operator::Eq, "");
        assert!(
            d("n")
                .operators(&cols, DriverKind::Postgres)
                .contains(&Operator::Between)
        );
        assert!(
            !d("n")
                .operators(&cols, DriverKind::Postgres)
                .contains(&Operator::Like)
        );
        assert_eq!(
            d("b").operators(&cols, DriverKind::Mysql),
            &[Operator::IsNull, Operator::IsNotNull]
        );
    }

    #[test]
    fn between_requires_two_values() {
        let cols = [col("n", TypeCategory::Integer)];
        assert!(
            draft("n", Operator::Between, "1")
                .to_filter(&cols, DriverKind::Postgres)
                .is_err()
        );
        assert!(
            draft("n", Operator::Between, "1,")
                .to_filter(&cols, DriverKind::Postgres)
                .is_err()
        );
        let f = draft("n", Operator::Between, "1, 5")
            .to_filter(&cols, DriverKind::Postgres)
            .unwrap();
        assert_eq!(f.values, ["1", "5"]);
        let f = draft("n", Operator::In, "1, 2,,3")
            .to_filter(&cols, DriverKind::Postgres)
            .unwrap();
        assert_eq!(f.values, ["1", "2", "3"]);
        assert!(
            draft("n", Operator::IsNull, "")
                .to_filter(&cols, DriverKind::Postgres)
                .unwrap()
                .values
                .is_empty()
        );
    }

    #[test]
    fn labels_and_op_validation_and_set_column() {
        assert_eq!(label(Operator::StartsWith), "starts with");
        assert_eq!(label(Operator::IsNotNull), "IS NOT NULL");
        assert_eq!(label(Operator::Ne), "!=");
        let cols = [
            col("n", TypeCategory::Integer),
            col("b", TypeCategory::Binary),
        ];
        let pg = DriverKind::Postgres;
        assert!(draft("b", Operator::Eq, "x").to_filter(&cols, pg).is_err());
        let mut d = draft("n", Operator::Between, "1,2");
        d.set_column(&cols, pg, "b");
        assert_eq!((d.column.as_str(), d.op), ("b", Operator::IsNull));
    }
}
