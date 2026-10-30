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
fn home_empty_state_shows_watermark() {
    // Arrange
    let s = state(&[]);
    let mut done = false;
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            empty_state(ui, &s);
        }
    });
    // `run` waits for pending images, so the decode must be done by now.
    h.run();
    h.get_by_label("cRustDB watermark");

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
            empty_state(ui, &s);
        }
    });
    h.run();
    h.get_by_label("Este app é feito em Rust. Vindo do Python? Aprenda com o Desbravando Rust.");
    h.get_by_label(&format!("Leia um capítulo grátis {}", icon::ARROW_RIGHT));
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
            empty_state(ui, &s);
        }
    });
    h.run();
    h.get_by_label(&format!("Leia um capítulo grátis {}", icon::ARROW_RIGHT))
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
fn book_promo_stays_inside_window_at_min_height() {
    // Arrange
    let s = state(&[]);
    let mut done = false;
    let mut h = Harness::builder()
        .with_size(egui::vec2(800.0, 480.0))
        .build_ui(|ui| {
            if ready(ui, &mut done) {
                empty_state(ui, &s);
            }
        });
    h.run();

    // Act
    let r = h
        .get_by_label(&format!("Leia um capítulo grátis {}", icon::ARROW_RIGHT))
        .rect();

    // Assert
    assert!(r.max.y <= 480.0, "promo cut off: {r:?}");
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

/// alpha -> dev, beta -> prod, catalog [dev, prod].
fn label_two(s: &mut AppState) {
    s.config.connections[0].label = Some("dev".into());
    s.config.connections[1].label = Some("prod".into());
    s.config.labels = ["dev", "prod"]
        .map(|n| crate::storage::config::Label {
            name: n.into(),
            color: [1, 2, 3],
        })
        .to_vec();
}

#[test]
fn home_groups_connections_by_label() {
    // Arrange
    let mut s = state(&["alpha", "beta"]);
    label_two(&mut s);
    s.config.labels.reverse();
    let mut done = false;
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            show(ui, &s);
        }
    });

    // Act
    h.run();

    // Assert
    let y = |t: &str| {
        (h.get_all_by_label(t))
            .map(|n| n.rect().min.y)
            .fold(f32::MAX, f32::min)
    };
    assert!(y("prod") < y("dev"), "folders follow catalog order");
    assert!(y("beta") < y("alpha"));
}

#[test]
fn home_ungrouped_section_last() {
    // Arrange
    let mut s = state(&["alpha", "beta"]);
    label_two(&mut s);
    s.config.connections[0].label = None;
    let mut done = false;
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            show(ui, &s);
        }
    });

    // Act
    h.run();

    // Assert
    let y = |t: &str| {
        (h.get_all_by_label(t))
            .map(|n| n.rect().min.y)
            .fold(f32::MAX, f32::min)
    };
    assert!(y("prod") < y("Ungrouped"));
    assert!(y("beta") < y("alpha"));
}

#[test]
fn home_search_filters_by_host() {
    // Arrange
    let mut s = state(&["alpha", "beta"]);
    s.config.connections[1].host = "db.special.io".into();
    let mut done = false;
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            show(ui, &s);
        }
    });
    h.ctx
        .data_mut(|d| d.insert_temp(search_id(), "SPECIAL".to_string()));

    // Act
    h.run();

    // Assert
    h.get_by_label("beta");
    assert!(h.query_by_label("alpha").is_none());
}

#[test]
fn home_filter_by_label_hides_others() {
    // Arrange
    let mut s = state(&["alpha", "beta"]);
    label_two(&mut s);
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
        .with_size(egui::vec2(1000.0, 800.0))
        .with_accessibility_check(false)
        .build_ui(|ui| {
            if ready(ui, &mut done) {
                empty_state(ui, &s);
            }
        });
    h.run();
    h.get_by_label(label).rect().size()
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

#[test]
fn home_list_has_no_logo_or_banner() {
    // Arrange
    let s = state(&["alpha"]);
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
    assert!(h.query_by_label("Desbravando Rust book cover").is_none());
}
