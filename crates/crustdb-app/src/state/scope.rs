//! Status bar scope text (`<label>/<database>`, #245).
use crustdb_core::model::DriverKind;

use super::workspace::WorkspaceState;

/// `<label or name>/<database>`, one rule per driver so every tab shows the same text:
/// Postgres/MySQL the database (active tab's, else the configured one), SQLite the file name,
/// Redis the logical database index.
pub(super) fn scope_of(w: &WorkspaceState) -> String {
    let head = w.config.label.as_deref().unwrap_or(&w.config.name);
    let tab_db = w.active_tab.and_then(|t| {
        w.tabs
            .iter()
            .find(|x| x.id == t)
            .map(|t| &t.object.database)
            .or_else(|| w.queries.iter().find(|q| q.id == t)?.database.as_ref())
    });
    let configured = w.config.database.as_deref();
    let place = match w.config.driver {
        DriverKind::Postgres | DriverKind::Mysql => tab_db.map(String::as_str).or(configured),
        DriverKind::Sqlite => configured.map(|f| {
            std::path::Path::new(f)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(f)
        }),
        DriverKind::Redis => Some(
            w.keyspace
                .as_ref()
                .map_or(configured.unwrap_or("0"), |k| k.current.as_str()),
        ),
    };
    match place {
        Some(p) => format!("{head}/{p}"),
        None => head.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn scope(driver: DriverKind, database: Option<&str>, label: Option<&str>) -> String {
        let mut s = crate::state::workspace::tests::state(&["alpha"]);
        s.open_workspace(s.config.connections[0].clone());
        let w = &mut s.workspaces[0];
        w.config.driver = driver;
        w.config.database = database.map(Into::into);
        w.config.label = label.map(Into::into);
        scope_of(w)
    }

    #[test]
    fn scope_follows_one_rule_per_driver() {
        // Arrange
        let cases = [
            (DriverKind::Postgres, Some("main"), "alpha/main"),
            (DriverKind::Mysql, Some("shop"), "alpha/shop"),
            (DriverKind::Sqlite, Some("/tmp/dir/app.db"), "alpha/app.db"),
            (DriverKind::Redis, Some("3"), "alpha/3"),
            (DriverKind::Redis, None, "alpha/0"),
            (DriverKind::Postgres, None, "alpha"),
        ];

        // Act & Assert
        for (d, db, want) in cases {
            assert_eq!(scope(d, db, None), want, "{d:?}");
        }
    }

    #[test]
    fn scope_prefers_label_over_name() {
        // Act & Assert
        assert_eq!(
            scope(DriverKind::Postgres, Some("main"), Some("production")),
            "production/main"
        );
    }

    #[test]
    fn scope_is_the_same_for_table_and_query_tabs() {
        // Arrange
        let mut s = crate::state::workspace::tests::state(&["alpha"]);
        let ws = s.open_workspace(s.config.connections[0].clone());
        s.workspaces[0].config.database = Some("main".into());
        let q = s.open_query(ws, Some("main".into()));
        let w = &mut s.workspaces[0];

        // Act
        let with_query = scope_of(w);
        w.active_tab = None;
        let without_tab = scope_of(w);

        // Assert
        assert!(q.is_some());
        assert_eq!(with_query, "alpha/main");
        assert_eq!(with_query, without_tab);
    }

    #[test]
    fn table_tab_database_counts_only_for_postgres_and_mysql() {
        // Arrange
        use crate::runtime::TabId;
        use crate::state::TableTab;
        use crustdb_core::model::{ObjectKind, ObjectRef};
        let cases = [
            (DriverKind::Postgres, Some("main"), "alpha/other"),
            (DriverKind::Mysql, Some("shop"), "alpha/other"),
            (DriverKind::Sqlite, Some("/tmp/app.db"), "alpha/app.db"),
            (DriverKind::Redis, Some("3"), "alpha/3"),
        ];

        // Act & Assert
        for (d, db, want) in cases {
            let mut s = crate::state::workspace::tests::state(&["alpha"]);
            s.open_workspace(s.config.connections[0].clone());
            let w = &mut s.workspaces[0];
            w.config.driver = d;
            w.config.database = db.map(Into::into);
            let object = ObjectRef {
                database: "other".into(),
                schema: Some("public".into()),
                name: "t".into(),
                kind: ObjectKind::Table,
            };
            w.tabs.push(TableTab::new(TabId(9), object, 300));
            w.active_tab = Some(TabId(9));
            assert_eq!(scope_of(w), want, "{d:?}");
        }
    }
}
