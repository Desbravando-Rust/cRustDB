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
            DriverKind::Mysql => (db, None),
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
        nodes.into_iter().filter_map(|n| self.expand(n)).collect()
    }

    /// Loaded tables of the configured database (same addressing as `expand_tables_of`).
    pub fn loaded_tables(&self, db: &str) -> &[ObjectRef] {
        let (db, schema) = self.tables_home(db);
        match self.entries.get(&Self::tables_folder(db, schema)) {
            Some(e) => match &e.load {
                Load::Done(v) => v,
                _ => &[],
            },
            None => &[],
        }
    }
}
