#![allow(clippy::unwrap_used, clippy::expect_used)]
use crustdb_core::model::{Cell, ColumnMeta, ForeignKeyInfo, ObjectKind, ObjectRef, TypeCategory};
use crustdb_core::query::{Filter, Operator};

use crate::runtime::{Command, RequestId, TabId, WorkspaceId};
use crate::state::workspace::tests::state;
use crate::state::{AppState, Structure, Tab, TableTab};

fn obj(name: &str) -> ObjectRef {
    ObjectRef {
        database: "d".into(),
        schema: None,
        name: name.into(),
        kind: ObjectKind::Table,
    }
}

fn col(name: &str) -> ColumnMeta {
    ColumnMeta {
        name: name.into(),
        type_name: "int4".into(),
        category: TypeCategory::Integer,
        nullable: Some(true),
        is_primary_key: false,
    }
}

fn text(v: &str) -> Cell {
    Cell::Text {
        value: v.into(),
        truncated: false,
    }
}

fn fk(cols: &[&str], target: &str, refs: &[&str]) -> ForeignKeyInfo {
    ForeignKeyInfo {
        name: "fk".into(),
        columns: cols.iter().map(|c| (*c).into()).collect(),
        ref_table: obj(target),
        ref_columns: refs.iter().map(|c| (*c).into()).collect(),
        on_update: "NO ACTION".into(),
        on_delete: "NO ACTION".into(),
    }
}

fn eq(column: &str, value: &str) -> Filter {
    Filter {
        column: column.into(),
        op: Operator::Eq,
        values: vec![value.into()],
    }
}

/// Workspace with an `orders` tab (columns `id`, `customer_id`, `region`) holding `row`.
fn setup(fks: Vec<ForeignKeyInfo>, row: Vec<Cell>) -> (AppState, WorkspaceId, TabId) {
    let mut s = state(&["a"]);
    let ws = s.open_workspace(s.config.connections[0].clone());
    s.open_table(ws, obj("orders"), RequestId(1));
    let t = s.workspace_mut(ws).unwrap().tabs.remove(0);
    let Tab::Table(mut t) = t;
    t.columns = vec![col("id"), col("customer_id"), col("region")];
    t.rows = vec![row.into()];
    t.structure = Some(Structure {
        fks,
        ..Structure::default()
    });
    let id = t.id;
    s.workspace_mut(ws).unwrap().tabs.push(Tab::Table(t));
    (s, ws, id)
}

fn table<'a>(s: &'a mut AppState, ws: WorkspaceId, name: &str) -> &'a mut TableTab {
    s.workspace_mut(ws)
        .unwrap()
        .tabs
        .iter_mut()
        .map(|Tab::Table(t)| t)
        .find(|t| t.object.name == name)
        .unwrap()
}

#[test]
fn follow_fk_opens_referenced_table_with_eq_filter() {
    // Arrange
    let row = vec![text("1"), text("7"), text("eu")];
    let (mut s, ws, orders) = setup(vec![fk(&["customer_id"], "customers", &["id"])], row);

    // Act
    let cmds = s.follow_fk(orders, 0, 1, RequestId(5));

    // Assert
    assert!(matches!(cmds[0], Command::OpenTable { .. }));
    assert!(matches!(cmds[1], Command::LoadStructure { .. }));
    let t = table(&mut s, ws, "customers");
    assert_eq!(t.filters, vec![eq("id", "7")]);
    assert!(t.filter_open);
    assert_eq!(t.drafts.len(), 1);
}

#[test]
fn follow_fk_focuses_open_tab_and_refetches_filtered() {
    // Arrange
    let row = vec![text("1"), text("7"), text("eu")];
    let (mut s, ws, orders) = setup(vec![fk(&["customer_id"], "customers", &["id"])], row);
    s.open_table(ws, obj("customers"), RequestId(2));
    table(&mut s, ws, "customers").columns = vec![col("id")];
    s.workspace_mut(ws).unwrap().active_tab = Some(orders);

    // Act
    let cmds = s.follow_fk(orders, 0, 1, RequestId(6));

    // Assert
    let [Command::FetchPage { query, .. }] = &cmds[..] else {
        panic!("expected one FetchPage");
    };
    assert_eq!(query.filters, vec![eq("id", "7")]);
    let t = table(&mut s, ws, "customers");
    let id = t.id;
    assert!(t.filter_open);
    assert_eq!(s.workspace_mut(ws).unwrap().active_tab, Some(id));
}

#[test]
fn follow_fk_composite_fk_adds_one_filter_per_column() {
    // Arrange
    let row = vec![text("1"), text("7"), text("eu")];
    let f = fk(&["customer_id", "region"], "customers", &["id", "region"]);
    let (mut s, ws, orders) = setup(vec![f], row);

    // Act
    s.follow_fk(orders, 0, 2, RequestId(5));

    // Assert
    let t = table(&mut s, ws, "customers");
    assert_eq!(t.filters, vec![eq("id", "7"), eq("region", "eu")]);
}

#[test]
fn follow_fk_ignores_non_fk_column_and_null_cell() {
    // Arrange
    let row = vec![text("1"), Cell::Null, text("eu")];
    let (mut s, ws, orders) = setup(vec![fk(&["customer_id"], "customers", &["id"])], row);

    // Act
    let on_plain_column = s.follow_fk(orders, 0, 0, RequestId(5));
    let on_null = s.follow_fk(orders, 0, 1, RequestId(6));

    // Assert
    assert!(on_plain_column.is_empty());
    assert!(on_null.is_empty());
    assert_eq!(s.workspace_mut(ws).unwrap().tabs.len(), 1);
}

#[test]
fn follow_fk_blocked_by_pending_changes_in_target() {
    // Arrange
    let row = vec![text("1"), text("7"), text("eu")];
    let (mut s, ws, orders) = setup(vec![fk(&["customer_id"], "customers", &["id"])], row);
    s.open_table(ws, obj("customers"), RequestId(2));
    let t = table(&mut s, ws, "customers");
    t.columns = vec![col("id")];
    t.pending = vec![(0, Some("9".into()))];

    // Act
    let cmds = s.follow_fk(orders, 0, 1, RequestId(6));

    // Assert
    assert!(cmds.is_empty());
    assert!(table(&mut s, ws, "customers").notice.is_some());
}
