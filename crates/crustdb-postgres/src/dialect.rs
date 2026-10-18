use crustdb_core::dialect::Dialect;
use crustdb_core::model::{CELL_MEMORY_CAP, ColumnMeta, DriverKind, ObjectRef, TypeCategory};

#[derive(Debug, Clone, Copy, Default)]
pub struct PgDialect;

impl Dialect for PgDialect {
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
    // type_name comes from the server catalog, never from user input.
    fn param(&self, index: usize, column: &ColumnMeta) -> String {
        format!("CAST(${index} AS {})", column.type_name)
    }
    fn select_expr(&self, c: &ColumnMeta) -> String {
        let id = self.quote_ident(&c.name);
        match c.category {
            TypeCategory::Text
            | TypeCategory::Json
            | TypeCategory::Binary
            | TypeCategory::Other => {
                format!("left({id}::text, {CELL_MEMORY_CAP}) AS {id}")
            }
            _ => format!("{id}::text AS {id}"),
        }
    }
    fn ci_like(&self, c: &ColumnMeta, param: &str) -> String {
        format!("{} ILIKE {param} ESCAPE '!'", self.as_text(c))
    }
    fn as_text(&self, c: &ColumnMeta) -> String {
        format!("{}::text", self.quote_ident(&c.name))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crustdb_core::dialect::build_table_query;
    use crustdb_core::driver::Param;
    use crustdb_core::model::ObjectKind;
    use crustdb_core::query::{Filter, Operator, TableQuery};

    fn col(name: &str, type_name: &str, category: TypeCategory) -> ColumnMeta {
        ColumnMeta {
            name: name.into(),
            type_name: type_name.into(),
            category,
            nullable: None,
            is_primary_key: false,
        }
    }

    #[test]
    fn quote_ident_doubles_quote_char() {
        assert_eq!(PgDialect.quote_ident(r#"we"ird Name"#), r#""we""ird Name""#);
    }

    #[test]
    fn pg_param_casts_text_to_column_type() {
        assert_eq!(
            PgDialect.param(1, &col("id", "int4", TypeCategory::Integer)),
            "CAST($1 AS int4)"
        );
    }

    #[test]
    fn pg_select_caps_text_transfer() {
        assert_eq!(
            PgDialect.select_expr(&col("body", "text", TypeCategory::Text)),
            r#"left("body"::text, 1048576) AS "body""#
        );
    }

    #[test]
    fn pg_select_non_text_casts_to_text() {
        assert_eq!(
            PgDialect.select_expr(&col("n", "int4", TypeCategory::Integer)),
            r#""n"::text AS "n""#
        );
    }

    #[test]
    fn pg_contains_uses_ilike() {
        assert_eq!(
            PgDialect.ci_like(&col("e", "text", TypeCategory::Text), "$1"),
            r#""e"::text ILIKE $1 ESCAPE '!'"#
        );
    }

    #[test]
    fn pg_as_text() {
        assert_eq!(
            PgDialect.as_text(&col("c", "int4", TypeCategory::Integer)),
            r#""c"::text"#
        );
    }

    fn obj(schema: Option<&str>) -> ObjectRef {
        ObjectRef {
            database: "db".into(),
            schema: schema.map(Into::into),
            name: "t".into(),
            kind: ObjectKind::Table,
        }
    }

    #[test]
    fn pg_qualified_name() {
        assert_eq!(
            PgDialect.qualified_name(&obj(Some("public"))),
            r#""public"."t""#
        );
        assert_eq!(PgDialect.qualified_name(&obj(None)), r#""t""#);
    }

    #[test]
    fn pg_contains_end_to_end() {
        let q = TableQuery {
            object: obj(Some("public")),
            columns: vec![col("e", "text", TypeCategory::Text)],
            filters: vec![Filter {
                column: "e".into(),
                op: Operator::Contains,
                values: vec!["50%".into()],
            }],
            sorts: vec![],
            limit: Some(10),
            offset: 0,
        };
        let b = build_table_query(&PgDialect, &q).unwrap();
        assert_eq!(
            b.sql,
            r#"SELECT left("e"::text, 1048576) AS "e" FROM "public"."t" WHERE "e"::text ILIKE CAST($1 AS text) ESCAPE '!' LIMIT 10 OFFSET 0"#
        );
        assert_eq!(b.params, vec![Param(Some("%50!%%".into()))]);
    }
}
