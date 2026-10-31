//! Loads unloaded nodes while a filter is active, so matches below collapsed nodes show up.
use crustdb_core::model::{DriverKind, ObjectKind};

use super::tree::{Load, NodePath, ObjectTree, folder_kinds};
use crate::runtime::Command;

impl ObjectTree {
    /// Requests every reachable node still `Idle`; call each frame, the cascade advances as
    /// results arrive. Empty with no filter.
    pub fn load_for_filter(&mut self) -> Vec<Command> {
        if self.filter_text().is_empty() {
            return vec![];
        }
        let dbs: Vec<String> = match &self.root {
            Load::Done(v) => v.iter().map(|d| d.name.clone()).collect(),
            _ => return vec![],
        };
        let mut paths = vec![];
        for d in dbs {
            let db = NodePath::Database(d.clone());
            if self.driver == DriverKind::Postgres {
                paths.push(db.clone());
                let schemas: Vec<_> = self.items(&db).iter().map(|s| s.name.clone()).collect();
                for s in schemas {
                    paths.extend(folder_paths(self.driver, &d, Some(s)));
                }
            } else {
                paths.extend(folder_paths(self.driver, &d, None));
            }
        }
        let mut cmds = vec![];
        for p in paths {
            let idle = self
                .entries
                .get(&p)
                .is_none_or(|e| matches!(e.load, Load::Idle));
            if idle && let Some(c) = self.request(&p) {
                cmds.push(c);
            }
        }
        cmds
    }
}

fn folder_paths(driver: DriverKind, d: &str, schema: Option<String>) -> Vec<NodePath> {
    folder_kinds(driver)
        .iter()
        .map(|&kind: &ObjectKind| NodePath::Folder {
            database: d.to_owned(),
            schema: schema.clone(),
            kind,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::runtime::WorkspaceId;
    use crustdb_core::model::ObjectRef;

    fn obj(schema: Option<&str>, name: &str, kind: ObjectKind) -> ObjectRef {
        ObjectRef {
            database: "app".into(),
            schema: schema.map(Into::into),
            name: name.into(),
            kind,
        }
    }

    fn tree(driver: DriverKind) -> ObjectTree {
        let mut t = ObjectTree::new(WorkspaceId(1), driver);
        let Some(Command::LoadChildren { req, .. }) = t.load_roots() else {
            panic!("expected roots load");
        };
        t.apply_children(req, vec![obj(None, "app", ObjectKind::Database)]);
        t
    }

    fn apply(t: &mut ObjectTree, cmd: &Command, items: Vec<ObjectRef>) {
        let Command::LoadChildren { req, .. } = cmd else {
            panic!("expected LoadChildren");
        };
        t.apply_children(*req, items);
    }

    fn is_table(c: &Command) -> bool {
        matches!(
            c,
            Command::LoadChildren {
                kind: Some(ObjectKind::Table),
                ..
            }
        )
    }

    #[test]
    fn filter_requests_schemas_of_loaded_database() {
        // Arrange
        let mut t = tree(DriverKind::Postgres);
        t.filter("ord");

        // Act
        let cmds = t.load_for_filter();

        // Assert
        assert!(matches!(
            cmds.as_slice(),
            [Command::LoadChildren { kind: Some(ObjectKind::Schema), database, .. }] if database == "app"
        ));
    }

    #[test]
    fn filter_requests_folders_of_loaded_schema() {
        // Arrange
        let mut t = tree(DriverKind::Postgres);
        t.filter("ord");
        let cmds = t.load_for_filter();
        apply(
            &mut t,
            &cmds[0],
            vec![obj(None, "public", ObjectKind::Schema)],
        );

        // Act
        let cmds = t.load_for_filter();

        // Assert
        assert_eq!(cmds.len(), folder_kinds(DriverKind::Postgres).len());
        assert!(cmds.iter().any(is_table));
    }

    #[test]
    fn filter_shows_match_after_folder_load() {
        // Arrange
        let mut t = tree(DriverKind::Postgres);
        t.filter("ord");
        let cmds = t.load_for_filter();
        apply(
            &mut t,
            &cmds[0],
            vec![obj(None, "public", ObjectKind::Schema)],
        );
        let cmds = t.load_for_filter();
        let table = cmds.iter().find(|c| is_table(c)).unwrap();

        // Act
        apply(
            &mut t,
            table,
            vec![obj(Some("public"), "orders", ObjectKind::Table)],
        );

        // Assert
        assert!(t.visible().any(|n| n.label == "orders"));
    }

    fn assert_flat_folders(driver: DriverKind) {
        // Arrange
        let mut t = tree(driver);
        t.filter("ord");

        // Act
        let cmds = t.load_for_filter();

        // Assert
        let kinds: Vec<_> = cmds
            .iter()
            .map(|c| match c {
                Command::LoadChildren {
                    kind: Some(k),
                    database,
                    schema: None,
                    ..
                } if database == "app" => *k,
                other => panic!("unexpected command for {driver:?}: {other:?}"),
            })
            .collect();
        assert_eq!(kinds, folder_kinds(driver));
    }

    #[test]
    fn filter_loads_folders_for_mysql() {
        assert_flat_folders(DriverKind::Mysql);
    }

    #[test]
    fn filter_loads_folders_for_sqlite() {
        assert_flat_folders(DriverKind::Sqlite);
    }

    #[test]
    fn filter_does_not_rerequest_after_empty_response() {
        // Arrange
        let mut t = tree(DriverKind::Postgres);
        t.filter("x");
        let cmds = t.load_for_filter();
        apply(&mut t, &cmds[0], vec![]);
        assert!(matches!(
            t.entries.get(&NodePath::Database("app".into())).map(|e| &e.load),
            Some(Load::Done(v)) if v.is_empty()
        ));

        // Act
        let again = t.load_for_filter();

        // Assert
        assert!(again.is_empty());
    }

    #[test]
    fn no_filter_is_noop() {
        // Arrange
        let mut t = tree(DriverKind::Postgres);

        // Act
        let cmds = t.load_for_filter();

        // Assert
        assert!(cmds.is_empty());
    }

    #[test]
    fn pending_nodes_are_not_rerequested() {
        // Arrange
        let mut t = tree(DriverKind::Postgres);
        t.filter("x");
        assert_eq!(t.load_for_filter().len(), 1);

        // Act
        let again = t.load_for_filter();

        // Assert
        assert!(again.is_empty());
    }
}
