#![allow(clippy::unwrap_used, clippy::expect_used)]
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::tests::ready;
use super::*;
use crate::state::workspace::tests::state;

#[test]
fn book_promo_shows_rust_paragraph_and_link() {
    // Arrange
    let s = state(&[]);
    let mut done = false;
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            empty_state(ui, &s);
        }
    });

    // Act
    h.run();

    // Assert
    assert!(h.query_by_label_contains("sem garbage collector").is_some());
    h.get_by_label(&format!("Leia um capítulo grátis {}", icon::ARROW_RIGHT));
}

#[test]
fn book_promo_shows_feed_chapter_when_loaded() {
    // Arrange
    let s = state(&[]);
    let _ = s.news.chapter.set(crate::news::Chapter {
        title: "Ownership".into(),
        url: "https://x/own".into(),
    });
    let mut done = false;
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            empty_state(ui, &s);
        }
    });

    // Act
    h.run();

    // Assert
    h.get_by_label(&format!("Ownership {}", icon::ARROW_RIGHT));
    assert!(h.query_by_label_contains("sem garbage collector").is_none());
}
