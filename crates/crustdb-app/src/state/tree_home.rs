//! Tree helpers for the workspace home view (kept apart so `tree.rs` stays small).
use crustdb_core::model::{DriverKind, ObjectKind, ObjectRef};

use super::tree::{Load, NodePath, ObjectTree};
use crate::runtime::Command;

impl ObjectTree {
    /// Database and schema that hold the tables of the configured database.
    fn tables_home<'a>(&self, db: &'a str) -> (&'a str, Option<String>) {
        match self.driver {
            // ponytail: only `public`; take the schema from the config if asked.
            DriverKind::Postgres => (db, Some("public".into())),
            DriverKind::Mysql | DriverKind::Redis => (db, None),
            DriverKind::Sqlite => ("main", None),
        }
    }

    fn tables_folder(db: &str, schema: Option<String>) -> NodePath {
        NodePath::Folder {
            database: db.into(),
            schema,
            kind: ObjectKind::Table,
        }
    }

    /// Expands the configured database down to its tables folder and returns the load commands.
    pub fn expand_tables_of(&mut self, db: &str) -> Vec<Command> {
        let (db, schema) = self.tables_home(db);
        let mut nodes = vec![NodePath::Database(db.into())];
        if let Some(s) = &schema {
            nodes.push(NodePath::Schema(db.into(), s.clone()));
        }
        nodes.push(Self::tables_folder(db, schema));
        // Only nodes never seen: an unrelated refresh must not re-expand what the user collapsed.
        nodes.retain(|n| !self.entries.contains_key(n));
        nodes.into_iter().filter_map(|n| self.expand(n)).collect()
    }

    /// Node whose load fills the home table list (refresh it to retry a failure).
    pub fn home_tables_node(&self, db: &str) -> NodePath {
        let (db, schema) = self.tables_home(db);
        Self::tables_folder(db, schema)
    }

    /// Error of the last tables load of the configured database, if it failed.
    pub fn tables_error(&self, db: &str) -> Option<&str> {
        match &self.entries.get(&self.home_tables_node(db))?.load {
            Load::Failed(m) => Some(m),
            _ => None,
        }
    }

    /// Loaded tables of the configured database (same addressing as `expand_tables_of`).
    /// `None` until the load finishes; callers must request it (`expand_tables_of`) after a connect
    /// or refresh, otherwise it stays `None`.
    pub fn loaded_tables(&self, db: &str) -> Option<&[ObjectRef]> {
        let (db, schema) = self.tables_home(db);
        match &self.entries.get(&Self::tables_folder(db, schema))?.load {
            Load::Done(v) => Some(v),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::runtime::WorkspaceId;

    fn tables_load(driver: DriverKind, db: &str) -> (String, Option<String>) {
        let mut t = ObjectTree::new(WorkspaceId(1), driver);
        let cmds = t.expand_tables_of(db);
        match cmds.last() {
            Some(Command::LoadChildren {
                database,
                schema,
                kind: Some(ObjectKind::Table),
                ..
            }) => (database.clone(), schema.clone()),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn postgres_loads_tables_of_public_schema() {
        // Act & Assert
        assert_eq!(
            tables_load(DriverKind::Postgres, "d"),
            ("d".into(), Some("public".into()))
        );
    }

    #[test]
    fn mysql_loads_tables_without_schema() {
        // Act & Assert
        assert_eq!(tables_load(DriverKind::Mysql, "d"), ("d".into(), None));
    }

    #[test]
    fn sqlite_loads_tables_of_main_ignoring_file_path() {
        // Act & Assert
        assert_eq!(
            tables_load(DriverKind::Sqlite, "/x/y.db"),
            ("main".into(), None)
        );
    }
}
