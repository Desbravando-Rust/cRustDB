#![allow(clippy::unwrap_used, clippy::expect_used)]
use crustdb_core::model::{Cell, ColumnMeta, ObjectKind, ObjectRef, TypeCategory};

use crate::runtime::{Command, RequestId, TabId};
use crate::state::workspace::tests::state;
use crate::state::{AppState, Dialog, Tab};

/// Workspace with a keyed `t(id)` tab of two rows, both selected.
fn setup() -> (AppState, TabId) {
    let mut s = state(&["a"]);
    let ws = s.open_workspace(s.config.connections[0].clone());
    let object = ObjectRef {
        database: "d".into(),
        schema: None,
        name: "t".into(),
        kind: ObjectKind::Table,
    };
    s.open_table(ws, object, RequestId(1));
    let Some(Tab::Table(t)) = s.workspace_mut(ws).unwrap().tabs.first_mut() else {
        panic!("no tab");
    };
    t.columns = vec![ColumnMeta {
        name: "id".into(),
        type_name: "int4".into(),
        category: TypeCategory::Integer,
        nullable: None,
        is_primary_key: true,
    }];
    t.rows = ["1", "2"]
        .map(|v| {
            vec![Cell::Text {
                value: v.into(),
                truncated: false,
            }]
            .into()
        })
        .into();
    t.selected_row = Some(1);
    t.select_anchor = Some(0);
    let id = t.id;
    (s, id)
}

#[test]
fn request_delete_rows_pushes_dialog_with_count() {
    // Arrange
    let (mut s, tab) = setup();

    // Act
    s.request_delete_rows(tab);

    // Assert
    assert!(matches!(
        s.dialogs.top(),
        Some(Dialog::DeleteRows { count: 2, .. })
    ));
}

#[test]
fn confirm_sends_delete() {
    // Arrange
    let (mut s, tab) = setup();
    s.request_delete_rows(tab);

    // Act
    let cmd = s.confirm_delete_rows(RequestId(9));

    // Assert
    assert!(matches!(
        cmd,
        Some(Command::UpdateCell {
            req: RequestId(9),
            ..
        })
    ));
    assert!(s.dialogs.top().is_none());
}

#[test]
fn cancel_sends_nothing() {
    // Arrange
    let (mut s, tab) = setup();
    s.request_delete_rows(tab);

    // Act
    let cmd = s.dialogs.cancel();

    // Assert
    assert!(cmd.is_none());
    assert!(s.table_mut(tab).unwrap().edit_req.is_none());
}

#[test]
fn selection_changed_between_request_and_confirm_sends_nothing() {
    // Arrange
    let (mut s, tab) = setup();
    s.request_delete_rows(tab);
    s.table_mut(tab).unwrap().select_anchor = None;

    // Act
    let cmd = s.confirm_delete_rows(RequestId(9));

    // Assert
    assert!(cmd.is_none());
    let t = s.table_mut(tab).unwrap();
    assert!(t.edit_req.is_none());
    assert_eq!(
        t.notice.as_deref(),
        Some("Selection changed, delete cancelled")
    );
}
