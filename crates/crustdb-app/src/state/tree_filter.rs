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

    fn pg_tree() -> ObjectTree {
        let mut t = ObjectTree::new(WorkspaceId(1), DriverKind::Postgres);
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

    #[test]
    fn filter_loads_unloaded_nodes() {
        // Arrange
        let mut t = pg_tree();
        t.filter("ord");

        // Act
        let cmds = t.load_for_filter();

        // Assert
        assert!(matches!(
            cmds.as_slice(),
            [Command::LoadChildren { kind: Some(ObjectKind::Schema), database, .. }] if database == "app"
        ));
        apply(
            &mut t,
            &cmds[0],
            vec![obj(None, "public", ObjectKind::Schema)],
        );
        let cmds = t.load_for_filter();
        assert_eq!(cmds.len(), folder_kinds(DriverKind::Postgres).len());
        let table = cmds
            .iter()
            .find(|c| {
                matches!(
                    c,
                    Command::LoadChildren {
                        kind: Some(ObjectKind::Table),
                        ..
                    }
                )
            })
            .unwrap();
        apply(
            &mut t,
            table,
            vec![obj(Some("public"), "orders", ObjectKind::Table)],
        );
        assert!(t.visible().any(|n| n.label == "orders"));
    }

    #[test]
    fn filter_load_is_noop_without_filter_or_when_loaded() {
        // Arrange
        let mut t = pg_tree();

        // Act & Assert
        assert!(t.load_for_filter().is_empty());
        t.filter("x");
        assert_eq!(t.load_for_filter().len(), 1);
        assert!(t.load_for_filter().is_empty());
    }
}
