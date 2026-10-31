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
    use crustdb_core::edit::{build_delete, build_insert};
    use crustdb_core::model::Cell;
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

    fn tbl() -> ObjectRef {
        ObjectRef {
            database: "db".into(),
            schema: None,
            name: "t".into(),
            kind: ObjectKind::Table,
        }
    }

    fn pk_col() -> ColumnMeta {
        ColumnMeta {
            is_primary_key: true,
            ..col("id", TypeCategory::Integer)
        }
    }

    #[test]
    fn sqlite_insert_sql() {
        // Arrange
        let cols = [pk_col(), col("n", TypeCategory::Text)];

        // Act
        let b = build_insert(&SqliteDialect, &tbl(), &cols, &[(1, Some("x".into()))]).unwrap();

        // Assert
        assert_eq!(b.sql, r#"INSERT INTO "db"."t" ("n") VALUES (?1)"#);
    }

    #[test]
    fn sqlite_delete_sql() {
        // Arrange
        let row = [Cell::Text {
            value: "1".into(),
            truncated: false,
        }];

        // Act
        let b = build_delete(&SqliteDialect, &tbl(), &[pk_col()], &[&row]).unwrap();

        // Assert
        assert_eq!(b.sql, r#"DELETE FROM "db"."t" WHERE ("id" = ?1)"#);
    }
}
