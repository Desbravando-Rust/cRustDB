use crustdb_core::dialect::Dialect;
use crustdb_core::model::{CELL_MEMORY_CAP, ColumnMeta, DriverKind, ObjectRef, TypeCategory};

#[derive(Debug, Clone, Copy, Default)]
pub struct SqliteDialect;

impl Dialect for SqliteDialect {
    fn kind(&self) -> DriverKind {
        DriverKind::Sqlite
    }
    fn quote_ident(&self, s: &str) -> String {
        format!("\"{}\"", s.replace('"', "\"\""))
    }
    // No schema level: the attached database ("main") is the qualifier.
    fn qualified_name(&self, o: &ObjectRef) -> String {
        if o.database.is_empty() {
            self.quote_ident(&o.name)
        } else {
            format!(
                "{}.{}",
                self.quote_ident(&o.database),
                self.quote_ident(&o.name)
            )
        }
    }
    fn param(&self, index: usize, _column: &ColumnMeta) -> String {
        format!("?{index}")
    }
    // ponytail: Binary is not capped (substr would turn non-blob values into text); cap it
    // when large blobs in the grid show up as a memory problem.
    fn select_expr(&self, c: &ColumnMeta) -> String {
        let id = self.quote_ident(&c.name);
        match c.category {
            TypeCategory::Text | TypeCategory::Other => {
                format!("substr(CAST({id} AS TEXT), 1, {CELL_MEMORY_CAP}) AS {id}")
            }
            _ => id,
        }
    }
    fn ci_like(&self, c: &ColumnMeta, param: &str) -> String {
        format!("LOWER({}) LIKE LOWER({param}) ESCAPE '!'", self.as_text(c))
    }
    fn as_text(&self, c: &ColumnMeta) -> String {
        format!("CAST({} AS TEXT)", self.quote_ident(&c.name))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crustdb_core::model::ObjectKind;

    fn col(name: &str, category: TypeCategory) -> ColumnMeta {
        ColumnMeta {
            name: name.into(),
            type_name: String::new(),
            category,
            nullable: None,
            is_primary_key: false,
        }
    }

    #[test]
    fn sqlite_param_is_numbered() {
        // Act & Assert
        assert_eq!(
            SqliteDialect.param(3, &col("id", TypeCategory::Integer)),
            "?3"
        );
    }

    #[test]
    fn sqlite_contains_lowercases_both_sides() {
        // Act & Assert
        assert_eq!(
            SqliteDialect.ci_like(&col("e", TypeCategory::Text), "?1"),
            "LOWER(CAST(\"e\" AS TEXT)) LIKE LOWER(?1) ESCAPE '!'"
        );
    }

    #[test]
    fn sqlite_select_caps_text() {
        // Act & Assert
        assert_eq!(
            SqliteDialect.select_expr(&col("t", TypeCategory::Text)),
            "substr(CAST(\"t\" AS TEXT), 1, 1048576) AS \"t\""
        );
        assert_eq!(
            SqliteDialect.select_expr(&col("n", TypeCategory::Integer)),
            "\"n\""
        );
    }

    #[test]
    fn sqlite_qualified_name() {
        // Arrange
        let o = ObjectRef {
            database: "main".into(),
            schema: None,
            name: "t".into(),
            kind: ObjectKind::Table,
        };

        // Act & Assert
        assert_eq!(SqliteDialect.qualified_name(&o), "\"main\".\"t\"");
    }

    #[test]
    fn sqlite_keyset_mixed_order_composite_key() {
        // Arrange
        let mut id = col("id", TypeCategory::Integer);
        id.is_primary_key = true;
        let mut name = col("name", TypeCategory::Text);
        name.is_primary_key = true;
        let q = crustdb_core::query::TableQuery {
            object: ObjectRef {
                database: "main".into(),
                schema: None,
                name: "t".into(),
                kind: ObjectKind::Table,
            },
            columns: vec![id, name],
            filters: vec![],
            sorts: vec![
                crustdb_core::query::Sort {
                    column: "name".into(),
                    descending: true,
                },
                crustdb_core::query::Sort {
                    column: "id".into(),
                    descending: false,
                },
            ],
            limit: Some(8),
            offset: 0,
            after: Some(vec!["b".into(), "5".into()]),
        };

        // Act
        let b = crustdb_core::dialect::build_table_query(&SqliteDialect, &q).unwrap();

        // Assert
        assert!(
            b.sql.contains(
                r#"(("main"."t"."name" < ?1) OR ("main"."t"."name" = ?2 AND "main"."t"."id" > ?3))"#
            ),
            "{}",
            b.sql
        );
        assert!(!b.sql.contains("OFFSET"));
        assert_eq!(b.params.len(), 3);
    }
}
