#![allow(clippy::unwrap_used, clippy::expect_used)]
use crustdb_core::model::{Cell, ObjectKind, ObjectRef};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use super::*;
use crate::runtime::{RequestId, TabId};

fn tab() -> TableTab {
    let object = ObjectRef {
        database: "d".into(),
        schema: None,
        name: "t".into(),
        kind: ObjectKind::Table,
    };
    let mut t = TableTab::new(TabId(1), object, 300);
    t.rows = vec![vec![Cell::text_capped("1")].into()];
    t
}

fn export_disabled(t: &mut TableTab) -> bool {
    let mut h = Harness::new_ui(|ui| {
        crate::ui::theme::install_theme(ui.ctx());
        drop(show(ui, t));
    });
    h.run();
    h.get_by_label(EXPORT_LABEL).accesskit_node().is_disabled()
}

#[test]
fn export_button_emits_intent() {
    // Arrange
    let mut t = tab();
    let mut out = vec![];
    let mut h = Harness::new_ui(|ui| {
        crate::ui::theme::install_theme(ui.ctx());
        out.extend(show(ui, &mut t));
    });
    h.run();

    // Act
    h.get_by_label(EXPORT_LABEL).click();
    h.run();
    drop(h);

    // Assert
    assert!(
        out.iter()
            .any(|i| matches!(i, Intent::ExportTable(TabId(1))))
    );
}

#[test]
fn export_button_disabled_without_rows() {
    // Arrange
    let mut t = tab();
    t.rows.clear();

    // Act & Assert
    assert!(export_disabled(&mut t));
}

#[test]
fn export_button_disabled_while_exporting() {
    // Arrange
    let mut t = tab();
    t.export_req = Some(RequestId(7));

    // Act & Assert
    assert!(export_disabled(&mut t));
}
