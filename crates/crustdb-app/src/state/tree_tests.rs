#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

fn obj(db: &str, schema: Option<&str>, name: &str, kind: ObjectKind) -> ObjectRef {
    ObjectRef {
        database: db.into(),
        schema: schema.map(Into::into),
        name: name.into(),
        kind,
    }
}

fn req_of(cmd: Option<Command>) -> RequestId {
    match cmd {
        Some(Command::LoadChildren { req, .. }) => req,
        other => panic!("expected LoadChildren, got {other:?}"),
    }
}

fn tree(driver: DriverKind) -> ObjectTree {
    let mut t = ObjectTree::new(WorkspaceId(1), driver);
    let r = req_of(t.load_roots());
    t.apply_children(r, vec![obj("app", None, "app", ObjectKind::Database)]);
    t
}

fn folder(schema: Option<&str>, kind: ObjectKind) -> NodePath {
    NodePath::Folder {
        database: "app".into(),
        schema: schema.map(Into::into),
        kind,
    }
}

fn labels(t: &ObjectTree) -> Vec<(usize, String)> {
    t.visible().map(|n| (n.depth, n.label)).collect()
}

#[test]
fn pg_tree_has_schema_level_mysql_does_not() {
    // Arrange
    let mut pg = tree(DriverKind::Postgres);

    // Act & Assert
    let r = req_of(pg.expand(NodePath::Database("app".into())));
    pg.apply_children(r, vec![obj("app", None, "public", ObjectKind::Schema)]);
    assert_eq!(labels(&pg), [(0, "app".into()), (1, "public".into())]);
    assert!(
        pg.expand(NodePath::Schema("app".into(), "public".into()))
            .is_none()
    );
    let v = labels(&pg);
    assert_eq!(v[2], (2, "Tables".into()));

    let mut my = tree(DriverKind::Mysql);
    assert!(my.expand(NodePath::Database("app".into())).is_none());
    assert_eq!(labels(&my)[1], (1, "Tables".into()));
}

#[test]
fn two_trees_never_share_request_ids() {
    // Arrange
    let mut a = ObjectTree::new(WorkspaceId(1), DriverKind::Mysql);
    let mut b = ObjectTree::new(WorkspaceId(2), DriverKind::Mysql);

    // Act & Assert
    let ra = req_of(a.load_roots());
    let rb = req_of(b.load_roots());
    assert_ne!(ra, rb);
    b.apply_children(ra, vec![obj("x", None, "x", ObjectKind::Database)]);
    assert!(b.visible().next().is_none(), "foreign reply ignored");
}

#[test]
fn empty_children_resolves_pending_and_root_refreshes() {
    // Arrange
    let mut t = ObjectTree::new(WorkspaceId(1), DriverKind::Mysql);

    // Act & Assert
    let r = req_of(t.load_roots());
    t.apply_children(r, vec![]);
    assert!(t.load_roots().is_none(), "empty reply resolved the root");
    let r2 = req_of(t.refresh_root());
    assert_ne!(r, r2);
    t.apply_children(r2, vec![obj("app", None, "app", ObjectKind::Database)]);
    assert_eq!(t.visible().count(), 1);
    t.reset();
    assert!(t.load_roots().is_some(), "reconnect starts over");
}

#[test]
fn filter_keeps_original_text() {
    // Arrange
    let mut t = ObjectTree::new(WorkspaceId(1), DriverKind::Mysql);

    // Act
    t.filter("ORD");

    // Assert
    assert_eq!(t.filter_text(), "ORD");
}

#[test]
fn folders_match_spec_per_driver() {
    use ObjectKind::*;

    // Act & Assert
    assert_eq!(
        folder_kinds(DriverKind::Postgres),
        [
            Table,
            View,
            MaterializedView,
            Function,
            Procedure,
            Trigger,
            Sequence,
            Type
        ]
    );
    assert_eq!(
        folder_kinds(DriverKind::Mysql),
        [Table, View, Function, Procedure, Trigger]
    );
}

#[test]
fn expand_loads_once_and_caches() {
    // Arrange
    let mut t = tree(DriverKind::Mysql);
    t.expand(NodePath::Database("app".into()));
    let f = folder(None, ObjectKind::Table);

    // Act & Assert
    let r = req_of(t.expand(f.clone()));
    assert!(t.expand(f.clone()).is_none(), "pending");
    t.apply_children(r, vec![obj("app", None, "users", ObjectKind::Table)]);
    assert!(t.toggle(f.clone()).is_none(), "collapse");
    assert!(t.toggle(f.clone()).is_none(), "re-expand uses the cache");
    assert!(t.visible().any(|n| n.label == "users"));
    let r2 = req_of(t.refresh(f));
    assert_ne!(r, r2);
}

#[test]
fn filter_matches_loaded_nodes_case_insensitive() {
    // Arrange
    let mut t = tree(DriverKind::Mysql);
    t.expand(NodePath::Database("app".into()));
    let r = req_of(t.expand(folder(None, ObjectKind::Table)));
    t.apply_children(
        r,
        vec![
            obj("app", None, "users", ObjectKind::Table),
            obj("app", None, "Orders", ObjectKind::Table),
        ],
    );

    // Act & Assert
    t.filter("ORD");
    let names: Vec<_> = t.visible().map(|n| n.label).collect();
    assert_eq!(names, ["app", "Tables", "Orders"]);
    t.filter("");
    assert_eq!(t.visible().filter(|n| n.object.is_some()).count(), 2);
}

#[test]
fn double_click_table_opens_data_tab_function_opens_ddl() {
    use ObjectKind::*;

    // Act & Assert
    for k in [Table, View, MaterializedView] {
        assert_eq!(open_action(k), OpenAction::Data);
    }
    for k in [Function, Procedure, Trigger, Sequence, Type] {
        assert_eq!(open_action(k), OpenAction::Ddl);
    }
}

#[test]
fn failed_load_shows_error_and_retries_on_first_click() {
    // Arrange
    let mut t = tree(DriverKind::Mysql);
    t.expand(NodePath::Database("app".into()));
    let f = folder(None, ObjectKind::Table);
    let r = req_of(t.expand(f.clone()));

    // Act
    t.fail_children(r, "boom".into());

    // Assert
    let row = t.visible().find(|n| n.label == "Tables").unwrap();
    assert_eq!(row.error.as_deref(), Some("boom"));
    assert!(!row.loading);
    let r2 = req_of(t.toggle(f));
    assert_ne!(r, r2, "first click on a failed node retries");
    assert!(t.visible().find(|n| n.label == "Tables").unwrap().loading);
}

#[test]
fn failed_roots_report_error_and_do_not_retry_by_themselves() {
    // Arrange
    let mut t = ObjectTree::new(WorkspaceId(1), DriverKind::Mysql);
    let r = req_of(t.load_roots());

    // Act
    t.fail_children(r, "down".into());

    // Assert
    assert_eq!(t.root_error(), Some("down"));
    assert!(t.load_roots().is_none());
    assert!(t.refresh_root().is_some());
    assert_eq!(t.root_error(), None);
}

#[test]
fn reload_loaded_rerequests_done_nodes_and_keeps_expanded() {
    // Arrange
    let mut t = tree(DriverKind::Mysql);
    let tables = folder(None, ObjectKind::Table);
    t.expand(NodePath::Database("app".into()));
    let r = req_of(t.expand(tables.clone()));
    t.apply_children(r, vec![obj("app", None, "a", ObjectKind::Table)]);

    // Act
    let cmds = t.reload_loaded();

    // Assert
    assert_eq!(cmds.len(), 2, "root and tables folder");
    for c in cmds {
        let Command::LoadChildren { req, kind, .. } = c else {
            panic!("expected LoadChildren");
        };
        let items = match kind {
            Some(ObjectKind::Table) => vec![
                obj("app", None, "a", ObjectKind::Table),
                obj("app", None, "b", ObjectKind::Table),
            ],
            _ => vec![obj("app", None, "app", ObjectKind::Database)],
        };
        t.apply_children(req, items);
    }
    assert_eq!(t.items(&tables).len(), 2);
    assert!(
        t.visible()
            .any(|n| n.path == Some(tables.clone()) && n.expanded)
    );
}
