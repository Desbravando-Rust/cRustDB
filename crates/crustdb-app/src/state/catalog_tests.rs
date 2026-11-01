#![allow(clippy::unwrap_used, clippy::expect_used)]
use crustdb_core::model::{ColumnDetail, ColumnMeta, ObjectKind, ObjectRef, TypeCategory};

use super::table_tab::TableView;
use super::workspace::tests::state;
use super::*;
use crate::runtime::{Command, Event, RequestId, TabId, WorkspaceId};

fn table() -> ObjectRef {
    ObjectRef {
        database: "d".into(),
        schema: None,
        name: "t".into(),
        kind: ObjectKind::Table,
    }
}

fn structure(req: u64) -> Event {
    Event::Structure {
        req: RequestId(req),
        columns: vec![ColumnDetail {
            meta: ColumnMeta {
                name: "id".into(),
                type_name: "int4".into(),
                category: TypeCategory::Integer,
                nullable: Some(false),
                is_primary_key: true,
            },
            default: None,
            comment: None,
        }],
        indexes: vec![],
        fks: vec![],
    }
}

/// A workspace with a loaded root, a cached column list and a query tab.
fn setup() -> (AppState, WorkspaceId, TabId) {
    let mut s = state(&["a"]);
    let ws = s.open_workspace(s.config.connections[0].clone());
    let w = s.workspace_mut(ws).unwrap();
    let Some(Command::LoadChildren { req, .. }) = w.tree.load_roots() else {
        panic!("expected LoadChildren");
    };
    w.tree.apply_children(req, vec![]);
    w.columns.insert(table(), vec!["id".into()]);
    let Some(Command::OpenEditor { tab, .. }) = s.open_query(ws, None) else {
        panic!("expected OpenEditor");
    };
    s.take_commands();
    (s, ws, tab)
}

#[test]
fn catalog_changed_reloads_tree_and_home_and_clears_column_cache() {
    // Arrange
    let (mut s, ws, tab) = setup();

    // Act
    s.apply(Event::CatalogChanged { tab });

    // Assert
    let cmds = s.take_commands();
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::LoadChildren { .. }))
    );
    assert!(
        s.workspace_mut(ws)
            .unwrap()
            .columns
            .columns_of("t")
            .is_empty()
    );
}

#[test]
fn catalog_changed_reloads_structure_of_clean_table_tabs_only() {
    // Arrange
    let (mut s, ws, query) = setup();
    s.open_table(ws, table(), RequestId(1));
    let mut other = table();
    other.name = "u".into();
    s.open_table(ws, other, RequestId(2));
    s.apply(structure(1));
    s.apply(structure(2));
    s.take_fetches(|| RequestId(3));
    let Some(Tab::Table(dirty)) = s.workspace_mut(ws).unwrap().tabs.get_mut(1) else {
        panic!("expected a table tab");
    };
    dirty.inserting = true;

    // Act
    s.apply(Event::CatalogChanged { tab: query });

    // Assert
    let mut n = 10;
    let cmds = s.take_fetches(|| {
        n += 1;
        RequestId(n)
    });
    let [Command::LoadStructure { tab, .. }] = &cmds[..] else {
        panic!("expected a single LoadStructure, got {cmds:?}");
    };
    assert_eq!(*tab, s.workspace_mut(ws).unwrap().tabs[0].id());
}

fn changed_tab_ids(s: &mut AppState, query: TabId) -> Vec<TabId> {
    s.apply(Event::CatalogChanged { tab: query });
    s.take_fetches(|| RequestId(100))
        .iter()
        .filter_map(|c| match c {
            Command::LoadStructure { tab, .. } => Some(*tab),
            _ => None,
        })
        .collect()
}

#[test]
fn catalog_changed_skips_ddl_view_tabs() {
    // Arrange
    let (mut s, ws, query) = setup();
    s.open_table(ws, table(), RequestId(1));
    let Some(Tab::Table(t)) = s.workspace_mut(ws).unwrap().tabs.get_mut(0) else {
        panic!("expected a table tab");
    };
    t.view = TableView::Ddl;

    // Act
    let reloaded = changed_tab_ids(&mut s, query);

    // Assert
    assert!(reloaded.is_empty());
}

#[test]
fn catalog_changed_skips_tabs_with_open_cell_editor() {
    // Arrange
    let (mut s, ws, query) = setup();
    s.open_table(ws, table(), RequestId(1));
    let Some(Tab::Table(t)) = s.workspace_mut(ws).unwrap().tabs.get_mut(0) else {
        panic!("expected a table tab");
    };
    t.editing = Some((0, 0, "x".into()));

    // Act
    let reloaded = changed_tab_ids(&mut s, query);

    // Assert
    assert!(reloaded.is_empty());
}
