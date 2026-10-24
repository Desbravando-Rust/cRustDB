use crustdb_core::dialect::Dialect;
use crustdb_core::model::{CELL_MEMORY_CAP, ColumnMeta, DriverKind, ObjectRef, TypeCategory};

#[derive(Debug, Clone, Copy, Default)]
pub struct MysqlDialect;

impl Dialect for MysqlDialect {
    fn kind(&self) -> DriverKind {
        DriverKind::Mysql
    }
    fn quote_ident(&self, s: &str) -> String {
        format!("`{}`", s.replace('`', "``"))
    }
    // MySQL has no schema level: the database is the qualifier.
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
    fn param(&self, _index: usize, _column: &ColumnMeta) -> String {
        "?".into()
    }
    fn select_expr(&self, c: &ColumnMeta) -> String {
        let id = self.quote_ident(&c.name);
        match c.category {
            TypeCategory::Text
            | TypeCategory::Json
            | TypeCategory::Binary
            | TypeCategory::Other => {
                format!("LEFT({id}, {CELL_MEMORY_CAP}) AS {id}")
            }
            _ => id,
        }
    }
    fn ci_like(&self, c: &ColumnMeta, param: &str) -> String {
        format!("LOWER({}) LIKE LOWER({param}) ESCAPE '!'", self.as_text(c))
    }
    fn as_text(&self, c: &ColumnMeta) -> String {
        format!("CAST({} AS CHAR)", self.quote_ident(&c.name))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crustdb_core::model::ObjectKind;

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
    fn mysql_quote_ident_doubles_backtick() {
        // Act & Assert
        assert_eq!(MysqlDialect.quote_ident("a`b"), "`a``b`");
    }

    #[test]
    fn mysql_param_is_question_mark() {
        // Act & Assert
        assert_eq!(
            MysqlDialect.param(3, &col("id", "int", TypeCategory::Integer)),
            "?"
        );
    }

    #[test]
    fn mysql_contains_lowercases_both_sides() {
        // Act & Assert
        assert_eq!(
            MysqlDialect.ci_like(&col("e", "varchar(10)", TypeCategory::Text), "?"),
            "LOWER(CAST(`e` AS CHAR)) LIKE LOWER(?) ESCAPE '!'"
        );
    }

    #[test]
    fn mysql_select_caps_text_and_binary() {
        // Act & Assert
        assert_eq!(
            MysqlDialect.select_expr(&col("b", "blob", TypeCategory::Binary)),
            "LEFT(`b`, 1048576) AS `b`"
        );
        assert_eq!(
            MysqlDialect.select_expr(&col("n", "int", TypeCategory::Integer)),
            "`n`"
        );
    }

    #[test]
    fn mysql_qualified_name() {
        // Arrange
        let o = |db: &str| ObjectRef {
            database: db.into(),
            schema: None,
            name: "t".into(),
            kind: ObjectKind::Table,
        };

        // Act & Assert
        assert_eq!(MysqlDialect.qualified_name(&o("db")), "`db`.`t`");
        assert_eq!(MysqlDialect.qualified_name(&o("")), "`t`");
    }
}
