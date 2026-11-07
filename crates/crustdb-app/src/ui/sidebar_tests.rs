#![allow(clippy::unwrap_used, clippy::expect_used)]
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use crustdb_core::model::{Product, ServerVersion};

use crate::runtime::Command;

use super::*;
use crate::state::Status;
use crate::state::workspace::tests::state;
use crate::ui::home::tests::ready;

#[test]
fn sidebar_failed_root_shows_error_and_retry() {
    // Arrange
    let mut s = online();
    let Some(Command::LoadChildren { req, .. }) = s.workspaces[0].tree.load_roots() else {
        panic!("expected roots load");
    };
    s.workspaces[0].tree.fail_children(req, "boom".into());
    let mut done = false;
    let mut got = vec![];
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            got.extend(show(ui, &s));
        }
    });
    h.run();

    // Act
    h.get_by_label("Retry").click();
    h.run();

    // Assert
    assert!(h.query_by_label_contains("boom").is_some());
    drop(h);
    assert!(got.contains(&Intent::RefreshRoot));
}

#[test]
fn sidebar_has_no_logo() {
    // Arrange
    let s = online();
    let mut done = false;
    let mut h = Harness::builder().build_ui(|ui| {
        if ready(ui, &mut done) {
            show(ui, &s);
        }
    });

    // Act
    h.run();

    // Assert
    h.get_by_label("New query");
    assert!(h.query_by_role(egui::accesskit::Role::Image).is_none());
    assert!(h.query_by_label("cRustDB logo").is_none());
}

#[test]
fn footer_shows_app_version() {
    // Arrange
    let s = state(&[]);
    let mut h = Harness::new_ui(|ui| footer(ui, &s));

    // Act
    h.run();

    // Assert
    assert!(
        h.query_by_label(concat!("v", env!("CARGO_PKG_VERSION")))
            .is_some()
    );
}

#[test]
fn footer_shows_update_link_when_newer_release() {
    // Arrange
    let s = state(&[]);
    let _ = s.news.latest.set("v999.0.0".into());
    let mut h = Harness::new_ui(|ui| footer(ui, &s));

    // Act
    h.run();

    // Assert
    h.get_by_label("Update available: v999.0.0");
}

fn online() -> AppState {
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
    s
}

#[test]
fn sidebar_shows_label_chip() {
    // Arrange
    let mut s = online();
    s.config.connections[0].label = Some("dev".into());
    s.config.labels = vec![crate::storage::config::Label {
        name: "dev".into(),
        color: [1, 2, 3],
    }];
    let mut done = false;
    let mut h = Harness::builder().build_ui(|ui| {
        if ready(ui, &mut done) {
            show(ui, &s);
        }
    });

    // Act
    h.run();

    // Assert
    h.get_by_label("dev");
}

/// (shows "new", shows "dev") after applying `f`.
fn chip_after(f: impl FnOnce(&mut AppState)) -> (bool, bool) {
    let mut s = online();
    s.workspaces[0].config.label = Some("dev".into());
    s.config.connections[0].label = Some("dev".into());
    f(&mut s);
    let mut done = false;
    let mut h = Harness::builder().build_ui(|ui| {
        if ready(ui, &mut done) {
            show(ui, &s);
        }
    });
    h.run();
    (
        h.query_by_label("new").is_some(),
        h.query_by_label("dev").is_some(),
    )
}

#[test]
fn sidebar_chip_follows_label_rename() {
    // Arrange & Act
    let shown = chip_after(|s| s.config.connections[0].label = Some("new".into()));

    // Assert
    assert_eq!(shown, (true, false));
}

#[test]
fn sidebar_chip_gone_after_label_delete() {
    // Arrange & Act
    let shown = chip_after(|s| s.config.connections[0].label = None);

    // Assert
    assert_eq!(shown, (false, false));
}

#[test]
fn sidebar_buttons_are_spaced() {
    // Arrange
    let s = online();
    let mut done = false;
    let mut h = Harness::builder().build_ui(|ui| {
        if ready(ui, &mut done) {
            show(ui, &s);
        }
    });

    // Act
    h.run();

    // Assert
    let new = h.get_by_label("New query").rect();
    let disc = h.get_by_label("Disconnect").rect();
    assert!(disc.left() - new.right() >= 8.0 + h.ctx.global_style().spacing.item_spacing.x);
}

#[test]
fn sidebar_filter_sits_in_gray_frame() {
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
    let mut h = Harness::builder().build_ui(|ui| {
        if ready(ui, &mut done) {
            show(ui, &s);
        }
    });

    // Act
    h.run();

    // Assert
    let input = h.get_by_role(egui::accesskit::Role::TextInput).rect();
    let framed = h.output().shapes.iter().any(|s| match &s.shape {
        egui::Shape::Rect(r) => {
            r.fill == egui::Color32::from_gray(40) && r.rect.contains_rect(input)
        }
        _ => false,
    });
    assert!(framed);
}

#[test]
fn sidebar_lists_open_connections_with_active_marked() {
    // Arrange
    let mut s = state(&["alpha", "beta"]);
    s.open_workspace(s.config.connections[0].clone());
    let other = s.open_workspace(s.config.connections[1].clone());
    s.workspaces[0].status = Status::Online(ServerVersion {
        product: Product::Postgres,
        major: 16,
        minor: 0,
        patch: 0,
        raw: "16.0".into(),
    });
    s.active = Some(s.workspaces[0].id);
    let mut done = false;
    let mut got = vec![];
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            got.extend(show(ui, &s));
        }
    });
    h.run();

    // Act
    h.get_by_label("beta").click();
    h.run();

    // Assert
    assert_eq!(
        h.get_by_label("alpha").accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
    drop(h);
    assert!(got.contains(&Intent::SwitchWorkspace(other)));

    // Act: clicking the active one
    let mut again = vec![];
    let mut done = false;
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            again.extend(show(ui, &s));
        }
    });
    h.run();
    h.get_by_label("alpha").click();
    h.run();
    drop(h);
    assert!(again.is_empty());
}

#[test]
fn sidebar_shows_chip_on_every_labeled_workspace() {
    // Arrange
    let mut s = state(&["alpha", "beta"]);
    s.open_workspace(s.config.connections[0].clone());
    s.open_workspace(s.config.connections[1].clone());
    s.config.connections[1].label = Some("prod".into());
    s.workspaces[0].status = Status::Online(ServerVersion {
        product: Product::Postgres,
        major: 16,
        minor: 0,
        patch: 0,
        raw: "16.0".into(),
    });
    s.active = Some(s.workspaces[0].id);
    let mut done = false;
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            show(ui, &s);
        }
    });

    // Act
    h.run();

    // Assert
    h.get_by_label("prod");
}

#[test]
fn folder_color_distinguishes_kinds() {
    // Arrange & Act & Assert
    use crustdb_core::model::ObjectKind as K;
    assert_ne!(folder_color(K::Table), folder_color(K::View));
    assert_eq!(folder_color(K::View), folder_color(K::MaterializedView));
    assert_eq!(folder_color(K::Function), folder_color(K::Procedure));
    assert_ne!(folder_color(K::Function), folder_color(K::Table));
    assert_eq!(folder_color(K::Sequence), egui::Color32::GRAY);
}

#[test]
fn sidebar_has_connections_and_explorer_headers() {
    // Arrange
    let s = online();
    let mut done = false;
    let mut got = vec![];
    let mut h = Harness::new_ui(|ui| {
        if ready(ui, &mut done) {
            got.extend(show(ui, &s));
        }
    });
    h.run();

    // Act
    h.get_by_label("New connection").click();
    h.run();

    // Assert
    h.get_by_label("CONNECTIONS");
    h.get_by_label("EXPLORER");
    drop(h);
    assert!(got.contains(&Intent::NewConnection));
}

#[test]
fn footer_shows_product_name() {
    // Arrange
    let s = state(&[]);
    let mut h = Harness::new_ui(|ui| footer(ui, &s));

    // Act
    h.run();

    // Assert
    h.get_by_label("cRustDB Desktop");
}
