#![allow(clippy::unwrap_used, clippy::expect_used)]
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::tests::ready;
use super::*;
use crate::state::workspace::tests::state;

fn empty_state_harness(s: &AppState, size: egui::Vec2) -> Harness<'_> {
    let mut done = false;
    let mut h = Harness::builder()
        .with_size(size)
        .with_accessibility_check(false)
        .build_ui(move |ui| {
            if ready(ui, &mut done) {
                empty_state(ui, s);
            }
        });
    h.run();
    h
}

#[test]
fn empty_state_text_without_connections() {
    // Arrange
    let s = state(&[]);

    // Act
    let h = empty_state_harness(&s, egui::vec2(1000.0, 800.0));

    // Assert
    h.get_by_label("Create a connection to get started");
}

#[test]
fn empty_state_text_with_connections() {
    // Arrange
    let s = state(&["alpha"]);

    // Act
    let h = empty_state_harness(&s, egui::vec2(1000.0, 800.0));

    // Assert
    h.get_by_label("Select a connection");
    h.get_by_label("Double-click to connect · Right-click for options");
}

#[test]
fn empty_state_hides_banner_then_watermark_when_small() {
    // Arrange
    let s = state(&[]);

    // Act
    let mid = empty_state_harness(&s, egui::vec2(500.0, 350.0));
    let tiny = empty_state_harness(&s, egui::vec2(300.0, 200.0));

    // Assert
    assert!(mid.query_by_label("Desbravando Rust book cover").is_none());
    mid.get_by_label("cRustDB watermark");
    assert!(tiny.query_by_label("cRustDB watermark").is_none());
    tiny.get_by_label("Create a connection to get started");
}
