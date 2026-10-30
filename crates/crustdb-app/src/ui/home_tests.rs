#![allow(clippy::unwrap_used, clippy::expect_used)]
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use crustdb_core::model::{Product, ServerVersion};

use super::*;
use crate::state::workspace::tests::state;

/// Installs theme fonts and image loaders on the first pass and returns
/// `false` then: fonts apply on the next pass, and without loaders the
/// logo renders egui's "⚠️" error glyph, which no installed font draws.
pub(crate) fn ready(ui: &egui::Ui, done: &mut bool) -> bool {
    if std::mem::replace(done, true) {
        return true;
    }
    crate::ui::theme::install_theme(ui.ctx());
    egui_extras::install_image_loaders(ui.ctx());
    false
}

#[test]
fn home_lists_saved_connections() {
    // Arrange
    let s = state(&["alpha", "beta"]);
    let mut done = false;
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            show(ui, &s);
        }
    });

    // Act
    h.run();

    // Assert
    h.get_by_label("alpha");
    h.get_by_label("beta");
}

#[test]
fn home_has_single_new_button() {
    // Arrange
    let s = state(&[]);
    let mut done = false;
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            show(ui, &s);
        }
    });

    // Act
    h.run();

    // Assert
    h.get_by_label(&format!("{} New", icon::PLUS));
    assert!(h.query_by_label_contains("Import URL").is_none());
}

#[test]
fn home_shows_logo() {
    // Arrange
    let s = state(&[]);
    let mut done = false;
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            show(ui, &s);
        }
    });
    // `run` waits for pending images, so the decode must be done by now.
    h.run();
    h.get_by_label("cRustDB logo");

    // Act
    let poll = h.ctx.try_load_image(
        "bytes://../../../../assets/imgs/logo2.png",
        egui::SizeHint::default(),
    );

    // Assert
    assert!(matches!(poll, Ok(egui::load::ImagePoll::Ready { .. })));
}

#[test]
fn home_shows_book_promo() {
    // Arrange
    let s = state(&[]);
    let mut done = false;
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            show(ui, &s);
        }
    });
    h.run();
    h.get_by_label(
        "This app is built in Rust. Coming from Python? Learn it with Desbravando Rust.",
    );
    h.get_by_label(&format!("Read a free chapter {}", icon::ARROW_RIGHT));
    h.get_by_label("Desbravando Rust book cover");

    // Act
    let poll = h.ctx.try_load_image(
        "bytes://../../../../assets/imgs/book-cover.png",
        egui::SizeHint::default(),
    );

    // Assert
    assert!(matches!(poll, Ok(egui::load::ImagePoll::Ready { .. })));
}

#[test]
fn book_promo_links_to_site() {
    // Arrange
    let s = state(&[]);
    let mut done = false;
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            show(ui, &s);
        }
    });
    h.run();
    h.get_by_label(&format!("Read a free chapter {}", icon::ARROW_RIGHT))
        .click();
    h.step();

    // Act
    let opened = h.output().platform_output.commands.iter().any(|c| {
        matches!(c, egui::OutputCommand::OpenUrl(u) if u.url == "https://desbravandorust.com.br/")
    });

    // Assert
    assert!(opened);
}

#[test]
fn book_promo_stays_visible_when_list_overflows() {
    // Arrange
    let names: Vec<String> = (0..15).map(|i| format!("conn{i}")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let s = state(&names);
    let mut done = false;
    let mut h = Harness::builder()
        .with_size(egui::vec2(800.0, 300.0))
        .build_ui(|ui| {
            if ready(ui, &mut done) {
                show(ui, &s);
            }
        });
    h.run();

    // Act
    let r = h
        .get_by_label(&format!("Read a free chapter {}", icon::ARROW_RIGHT))
        .rect();

    // Assert
    assert!(r.max.y <= 300.0, "promo cut off: {r:?}");
}

#[test]
fn double_click_connects() {
    // Arrange
    let s = state(&["alpha"]);
    let id = s.config.connections[0].id;
    let mut intents = vec![];
    let mut done = false;
    let mut h = Harness::builder().with_step_dt(0.05).build_ui(|ui| {
        if ready(ui, &mut done) {
            intents.extend(show(ui, &s));
        }
    });
    h.run();
    h.get_by_label("alpha").click();
    h.step();
    h.get_by_label("alpha").click();
    h.step();

    // Act
    h.run();
    drop(h);

    // Assert
    assert!(intents.contains(&Intent::Connect(id)));
}

#[test]
fn live_connection_shows_open_instead_of_connect() {
    // Arrange
    let mut s = state(&["alpha"]);
    let ws = s.open_workspace(s.config.connections[0].clone());
    s.workspaces[0].status = Status::Online(ServerVersion {
        product: Product::Postgres,
        major: 16,
        minor: 0,
        patch: 0,
        raw: "16.0".into(),
    });
    let mut intents = vec![];
    let mut done = false;
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            intents.extend(show(ui, &s));
        }
    });
    h.run();

    // Act & Assert
    h.get_by_label("Active");
    assert!(h.query_by_label("Connect").is_none());
    h.get_by_label("Open").click();
    h.run();
    drop(h);
    assert!(intents.contains(&Intent::SwitchWorkspace(ws)));
}

#[test]
fn home_hides_logo_when_connection_active() {
    // Arrange
    let mut s = state(&["alpha"]);
    let ws = s.open_workspace(s.config.connections[0].clone());
    s.workspaces[0].status = Status::Online(ServerVersion {
        product: Product::Postgres,
        major: 16,
        minor: 0,
        patch: 0,
        raw: "16.0".into(),
    });
    s.active = Some(ws);
    let mut done = false;
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            show(ui, &s);
        }
    });

    // Act
    h.run();

    // Assert
    assert!(h.query_by_label("cRustDB logo").is_none());
}

#[test]
fn home_filter_by_label_hides_others() {
    // Arrange
    let mut s = state(&["alpha", "beta"]);
    s.config.connections[0].labels = vec!["dev".into()];
    s.config.connections[1].labels = vec!["prod".into()];
    s.config.labels = ["dev", "prod"]
        .map(|n| crate::storage::config::Label {
            name: n.into(),
            color: [1, 2, 3],
        })
        .to_vec();
    let mut done = false;
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            show(ui, &s);
        }
    });
    h.ctx
        .data_mut(|d| d.insert_temp(labels::filter_id(), Some("dev".to_string())));

    // Act
    h.run();

    // Assert
    h.get_by_label("alpha");
    assert!(h.query_by_label("beta").is_none());
}

#[test]
fn home_filter_resets_when_label_deleted() {
    // Arrange
    let s = state(&["alpha"]);
    let mut done = false;
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            show(ui, &s);
        }
    });
    h.ctx
        .data_mut(|d| d.insert_temp(labels::filter_id(), Some("gone".to_string())));

    // Act
    h.run();

    // Assert
    h.get_by_label("alpha");
}

fn size_of(label: &str) -> egui::Vec2 {
    let s = state(&[]);
    let mut done = false;
    let mut h = Harness::builder()
        .with_size(egui::vec2(800.0, 600.0))
        .with_accessibility_check(false)
        .build_ui(|ui| {
            if ready(ui, &mut done) {
                show(ui, &s);
            }
        });
    h.run();
    h.get_by_label(label).rect().size()
}

#[test]
fn home_logo_is_150_square() {
    // Act & Assert
    assert_eq!(size_of("cRustDB logo"), egui::vec2(150.0, 150.0));
}

#[test]
fn book_cover_is_80x122() {
    // Act & Assert
    // The image keeps its aspect ratio (300x456 -> 80x121.6).
    let size = size_of("Desbravando Rust book cover");
    assert_eq!(size.x, 80.0);
    assert!((size.y - 122.0).abs() < 0.5);
}

#[test]
fn home_has_no_title_heading() {
    // Arrange
    let s = state(&[]);
    let mut done = false;
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            show(ui, &s);
        }
    });

    // Act
    h.run();

    // Assert
    assert!(h.query_by_label("cRustDB").is_none());
}

#[test]
fn new_button_is_right_aligned() {
    // Arrange
    let s = state(&[]);
    let mut done = false;
    let mut h = Harness::builder()
        .with_size(egui::vec2(800.0, 600.0))
        .build_ui(|ui| {
            if ready(ui, &mut done) {
                show(ui, &s);
            }
        });

    // Act
    h.run();

    // Assert
    let new = h.get_by_label(&format!("{} New", icon::PLUS)).rect();
    assert!(new.right() > 400.0);
}

#[test]
fn engine_colors_differ() {
    // Act & Assert
    assert_ne!(
        engine_color(DriverKind::Postgres),
        engine_color(DriverKind::Mysql)
    );
}
