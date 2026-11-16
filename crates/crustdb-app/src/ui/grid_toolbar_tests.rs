#![allow(clippy::unwrap_used, clippy::expect_used)]
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::tests::{staged, tab};
use super::*;
use crate::runtime::TabId;
use crate::ui::data_toolbar::{FILTERS_LABEL, RELOAD_LABEL};

#[test]
fn toolbar_filters_opens_filter_bar() {
    // Arrange
    let mut t = tab();
    t.editing = None;
    let mut h = Harness::builder().build_ui(|ui| {
        crate::ui::theme::install_theme(ui.ctx());
        drop(show(ui, &mut t));
    });
    h.run();

    // Act
    h.get_by_label(FILTERS_LABEL).click();
    h.run();
    drop(h);

    // Assert
    assert!(t.filter_open);
    assert!(!t.drafts.is_empty());
}

#[test]
fn toolbar_refresh_emits_retry() {
    // Arrange
    let mut t = tab();
    t.editing = None;
    let mut out = vec![];
    let mut h = Harness::new_ui(|ui| {
        crate::ui::theme::install_theme(ui.ctx());
        out.extend(show(ui, &mut t));
    });
    h.run();

    // Act
    h.get_by_label(RELOAD_LABEL).click();
    h.run();
    drop(h);

    // Assert
    assert!(out.iter().any(|i| matches!(i, Intent::Retry(TabId(1)))));
}

#[test]
fn footer_has_no_save_button_but_text_actions() {
    // Arrange
    let mut t = staged();
    let mut h = Harness::new_ui(|ui| {
        crate::ui::theme::install_theme(ui.ctx());
        drop(show(ui, &mut t));
    });

    // Act
    h.run();

    // Assert
    assert!(h.query_by_label("Save").is_none());
    assert!(h.query_by_label("Refresh").is_some());
    assert!(h.query_by_label("Add row").is_some());
}

#[test]
fn footer_text_buttons_emit_intents() {
    // Arrange
    let mut t = tab();
    t.editing = None;
    let mut out = vec![];
    let mut h = Harness::new_ui(|ui| {
        crate::ui::theme::install_theme(ui.ctx());
        out.extend(show(ui, &mut t));
    });
    h.run();

    // Act
    h.get_by_label("Refresh").click();
    h.run();
    h.get_by_label("Add row").click();
    h.run();
    drop(h);

    // Assert
    assert!(out.iter().any(|i| matches!(i, Intent::Retry(TabId(1)))));
    assert!(out.iter().any(|i| matches!(i, Intent::AddRow(TabId(1)))));
}
