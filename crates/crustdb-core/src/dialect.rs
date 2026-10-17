use crate::driver::Param;
use crate::model::{ColumnMeta, DriverKind, ObjectRef, TypeCategory};
use crate::query::{Arity, Operator, QueryError, TableQuery, operators_for};

pub trait Dialect: Send + Sync {
    fn kind(&self) -> DriverKind;
    fn quote_ident(&self, s: &str) -> String;
    fn qualified_name(&self, o: &ObjectRef) -> String;
    fn param(&self, index: usize, column: &ColumnMeta) -> String;
    fn select_expr(&self, column: &ColumnMeta) -> String;
    fn ci_like(&self, column: &ColumnMeta, param: &str) -> String;
    /// Expression casting the column to text (used by LIKE / NOT LIKE / ILIKE).
    fn as_text(&self, column: &ColumnMeta) -> String;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltQuery {
    pub sql: String,
    pub params: Vec<Param>,
}

/// Escape char is `!`; escapes `!`, `%` and `_`.
pub fn escape_like(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if matches!(c, '!' | '%' | '_') {
            out.push('!');
        }
        out.push(c);
    }
    out
}

pub fn build_table_query(d: &dyn Dialect, q: &TableQuery) -> Result<BuiltQuery, QueryError> {
    let find = |name: &str| {
        q.columns
            .iter()
            .find(|c| c.name == name)
            .ok_or_else(|| QueryError::UnknownColumn(name.to_string()))
    };
    // Pattern params are bound as text, not as the column type.
    let text = ColumnMeta {
        name: String::new(),
        type_name: "text".into(),
        category: TypeCategory::Text,
        nullable: None,
        is_primary_key: false,
    };

    let cols = if q.columns.is_empty() {
        "*".to_string()
    } else {
        q.columns
            .iter()
            .map(|c| d.select_expr(c))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut sql = format!("SELECT {cols} FROM {}", d.qualified_name(&q.object));

    let mut params: Vec<Param> = Vec::new();
    let mut conds = Vec::new();
    for f in &q.filters {
        let col = find(&f.column)?;
        if !operators_for(col.category, d.kind()).contains(&f.op) {
            return Err(QueryError::OperatorNotAllowed(f.op));
        }
        let n = f.values.len();
        let ok = match f.op.arity() {
            Arity::Zero => n == 0,
            Arity::One => n == 1,
            Arity::Two => n == 2,
            Arity::Many => n > 0,
        };
        if !ok {
            return Err(QueryError::Arity { op: f.op, got: n });
        }

        let id = d.quote_ident(&col.name);
        let mut bind = |v: String, meta: &ColumnMeta| {
            params.push(Param(Some(v)));
            d.param(params.len(), meta)
        };
        let v = f.values.first().cloned().unwrap_or_default();
        use Operator::*;
        let cond = match f.op {
            Eq | Ne | Lt | Gt | Le | Ge => {
                let op = match f.op {
                    Eq => "=",
                    Ne => "<>",
                    Lt => "<",
                    Gt => ">",
                    Le => "<=",
                    _ => ">=",
                };
                format!("{id} {op} {}", bind(v, col))
            }
            In | NotIn => {
                let ps: Vec<String> = f.values.iter().map(|v| bind(v.clone(), col)).collect();
                let not = if f.op == NotIn { "NOT " } else { "" };
                format!("{id} {not}IN ({})", ps.join(", "))
            }
            Between => {
                let a = bind(v, col);
                let b = bind(f.values[1].clone(), col);
                format!("{id} BETWEEN {a} AND {b}")
            }
            IsNull => format!("{id} IS NULL"),
            IsNotNull => format!("{id} IS NOT NULL"),
            Contains | NotContains | StartsWith | EndsWith => {
                let e = escape_like(&v);
                let pat = match f.op {
                    StartsWith => format!("{e}%"),
                    EndsWith => format!("%{e}"),
                    _ => format!("%{e}%"),
                };
                let like = d.ci_like(col, &bind(pat, &text));
                if f.op == NotContains {
                    format!("NOT ({like})")
                } else {
                    like
                }
            }
            Like | NotLike | ILike => {
                let kw = match f.op {
                    Like => "LIKE",
                    NotLike => "NOT LIKE",
                    _ => "ILIKE",
                };
                format!("{} {kw} {}", d.as_text(col), bind(v, &text))
            }
        };
        conds.push(cond);
    }
    if !conds.is_empty() {
        sql.push_str(&format!(" WHERE {}", conds.join(" AND ")));
    }

    // Qualified so ORDER BY hits the base column, not the `::text` output alias of the same name.
    // Json: base `json` has no ordering operator, so order by its text (an expression, which
    // also resolves to the input column).
    // ponytail: xml/geometric types (category Other) remain unsortable.
    let base = d.qualified_name(&q.object);
    let key = |c: &ColumnMeta| {
        if c.category == TypeCategory::Json {
            d.as_text(c)
        } else {
            format!("{base}.{}", d.quote_ident(&c.name))
        }
    };
    let order: Vec<String> = if q.sorts.is_empty() {
        q.columns
            .iter()
            .filter(|c| c.is_primary_key)
            .map(|c| format!("{} ASC", key(c)))
            .collect()
    } else {
        q.sorts
            .iter()
            .map(|s| {
                let c = find(&s.column)?;
                let dir = if s.descending { "DESC" } else { "ASC" };
                Ok(format!("{} {dir}", key(c)))
            })
            .collect::<Result<_, QueryError>>()?
    };
    if !order.is_empty() {
        sql.push_str(&format!(" ORDER BY {}", order.join(", ")));
    }
    if let Some(n) = q.limit {
        sql.push_str(&format!(" LIMIT {n} OFFSET {}", q.offset));
    }
    Ok(BuiltQuery { sql, params })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::model::ObjectKind;
    use crate::query::{Filter, Sort};

    struct TestDialect;
    impl Dialect for TestDialect {
        fn kind(&self) -> DriverKind {
            DriverKind::Postgres
        }
        fn quote_ident(&self, s: &str) -> String {
            format!("\"{}\"", s.replace('"', "\"\""))
        }
        fn qualified_name(&self, o: &ObjectRef) -> String {
            match &o.schema {
                Some(s) => format!("{}.{}", self.quote_ident(s), self.quote_ident(&o.name)),
                None => self.quote_ident(&o.name),
            }
        }
        fn param(&self, index: usize, _: &ColumnMeta) -> String {
            format!("${index}")
        }
        fn select_expr(&self, c: &ColumnMeta) -> String {
            self.quote_ident(&c.name)
        }
        fn ci_like(&self, c: &ColumnMeta, p: &str) -> String {
            format!("{} ILIKE {p} ESCAPE '!'", self.as_text(c))
        }
        fn as_text(&self, c: &ColumnMeta) -> String {
            format!("{}::text", self.quote_ident(&c.name))
        }
    }

    fn col(name: &str, cat: TypeCategory, pk: bool) -> ColumnMeta {
        ColumnMeta {
            name: name.into(),
            type_name: "t".into(),
            category: cat,
            nullable: None,
            is_primary_key: pk,
        }
    }

    fn tq(filters: Vec<Filter>, sorts: Vec<Sort>) -> TableQuery {
        TableQuery {
            object: ObjectRef {
                database: "db".into(),
                schema: Some("public".into()),
                name: "t".into(),
                kind: ObjectKind::Table,
            },
            columns: vec![
                col("id", TypeCategory::Integer, false),
                col("a", TypeCategory::Text, false),
                col("b", TypeCategory::Text, false),
            ],
            filters,
            sorts,
            limit: Some(301),
            offset: 0,
        }
    }

    fn filter(c: &str, op: Operator, v: &[&str]) -> Filter {
        Filter {
            column: c.into(),
            op,
            values: v.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn sort(c: &str, descending: bool) -> Sort {
        Sort {
            column: c.into(),
            descending,
        }
    }

    fn build(q: &TableQuery) -> BuiltQuery {
        build_table_query(&TestDialect, q).unwrap()
    }

    fn build_err(f: Filter) -> QueryError {
        build_table_query(&TestDialect, &tq(vec![f], vec![])).unwrap_err()
    }

    #[test]
    fn multi_sort_order_preserved() {
        let q = tq(vec![], vec![sort("b", true), sort("a", false)]);
        assert!(build(&q).sql.ends_with(
            r#"ORDER BY "public"."t"."b" DESC, "public"."t"."a" ASC LIMIT 301 OFFSET 0"#
        ));
    }

    #[test]
    fn default_sort_uses_pk() {
        let mut q = tq(vec![], vec![]);
        q.columns[0].is_primary_key = true;
        assert!(build(&q).sql.contains(r#"ORDER BY "public"."t"."id" ASC"#));
        assert!(!build(&tq(vec![], vec![])).sql.contains("ORDER BY"));
    }

    #[test]
    fn contains_escapes_wildcards() {
        assert_eq!(escape_like("50%_a!"), "50!%!_a!!");
        let b = build(&tq(
            vec![filter("a", Operator::NotContains, &["5%"])],
            vec![],
        ));
        assert!(
            b.sql
                .contains(r#"WHERE NOT ("a"::text ILIKE $1 ESCAPE '!')"#)
        );
        assert_eq!(b.params, vec![Param(Some("%5!%%".into()))]);
    }

    #[test]
    fn in_list_binds_each_value() {
        let b = build(&tq(
            vec![filter("id", Operator::In, &["1", "2", "3"])],
            vec![],
        ));
        assert_eq!(b.params.len(), 3);
        assert!(b.sql.contains(r#""id" IN ($1, $2, $3)"#));
    }

    #[test]
    fn params_numbered_across_filters() {
        let b = build(&tq(
            vec![
                filter("id", Operator::Between, &["1", "9"]),
                filter("a", Operator::Ne, &["x"]),
                filter("b", Operator::IsNull, &[]),
            ],
            vec![],
        ));
        assert!(
            b.sql
                .contains(r#"WHERE "id" BETWEEN $1 AND $2 AND "a" <> $3 AND "b" IS NULL"#)
        );
    }

    #[test]
    fn validation_errors() {
        assert!(matches!(
            build_err(filter("id", Operator::Between, &["1"])),
            QueryError::Arity { .. }
        ));
        assert!(matches!(
            build_err(filter("id", Operator::In, &[])),
            QueryError::Arity { .. }
        ));
        assert!(matches!(
            build_err(filter("zz", Operator::Eq, &["1"])),
            QueryError::UnknownColumn(_)
        ));
        assert!(matches!(
            build_err(filter("id", Operator::Contains, &["1"])),
            QueryError::OperatorNotAllowed(_)
        ));
        let q = tq(vec![], vec![sort("zz", false)]);
        assert!(build_table_query(&TestDialect, &q).is_err());
    }

    #[test]
    fn export_query_has_no_limit() {
        let mut q = tq(vec![], vec![]);
        q.limit = None;
        let sql = build(&q).sql;
        assert!(!sql.contains("LIMIT") && !sql.contains("OFFSET"));
    }

    #[test]
    fn empty_columns_selects_star() {
        let mut q = tq(vec![], vec![]);
        q.columns.clear();
        assert!(build(&q).sql.starts_with(r#"SELECT * FROM "public"."t""#));
    }
}
