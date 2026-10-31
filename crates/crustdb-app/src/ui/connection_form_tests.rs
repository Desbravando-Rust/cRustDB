#![allow(clippy::unwrap_used, clippy::expect_used)]
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::*;

#[test]
fn form_shows_keyring_hint_and_buttons() {
    // Arrange
    let mut f = ConnectionForm::new(DriverKind::Postgres, false);
    f.set_ssh(true);
    let mut h = Harness::builder()
        .with_accessibility_check(false) // text inputs and spin buttons are unnamed (#186)
        .build_ui(|ui| {
            show(ui, &mut f, &[]);
        });

    // Act
    h.run();

    // Assert
    h.get_by_label("Save");
    h.get_by_label("Test");
    h.get_by_label("OS keyring unavailable: passwords cannot be saved");
}

#[test]
fn url_box_shows_example_hint() {
    // Arrange
    let mut f = ConnectionForm::new(DriverKind::Postgres, true);
    let mut h = Harness::builder()
        .with_accessibility_check(false) // text inputs and spin buttons are unnamed (#186)
        .build_ui(|ui| {
            show(ui, &mut f, &[]);
        });

    // Act
    h.run();

    // Assert
    h.get_by_label("Import");
    h.get_by_label_contains("postgres://");
}

#[test]
fn sqlite_form_shows_only_file_fields() {
    // Arrange
    let mut f = ConnectionForm::new(DriverKind::Sqlite, true);
    let mut h = Harness::builder()
        .with_accessibility_check(false) // text inputs and spin buttons are unnamed (#186)
        .build_ui(|ui| {
            show(ui, &mut f, &[]);
        });
    h.run();
    h.get_by_label("Database file");
    h.get_by_label("Page size");

    // Act & Assert
    for hidden in ["Host", "Port", "User", "Password", "TLS mode", "SSH tunnel"] {
        assert!(h.query_by_label(hidden).is_none(), "{hidden} is shown");
    }
}

#[test]
fn sqlite_form_browse_emits_intent() {
    // Arrange
    let mut f = ConnectionForm::new(DriverKind::Sqlite, true);
    let mut intents = vec![];
    let mut h = Harness::builder()
        .with_accessibility_check(false) // text inputs and spin buttons are unnamed (#186)
        .build_ui(|ui| {
            intents.extend(show(ui, &mut f, &[]));
        });
    h.run();

    // Act
    h.get_by_label("Browse…").click();
    h.run();
    drop(h);

    // Assert
    assert!(intents.contains(&Intent::FormBrowseSqlite));
}

#[test]
fn buttons_reachable_in_small_window() {
    // Arrange
    let mut f = ConnectionForm::new(DriverKind::Postgres, true);
    f.set_ssh(true);
    let mut h = Harness::builder()
        .with_accessibility_check(false) // text inputs and spin buttons are unnamed (#186)
        .with_size(egui::vec2(400.0, 240.0))
        .build_ui(|ui| {
            show(ui, &mut f, &[]);
        });
    h.run();

    // Act
    h.get_by_label("Save").scroll_to_me();
    h.run();

    // Assert
    for label in ["Save", "Cancel"] {
        let max_y = h.get_by_label(label).rect().max.y;
        assert!(max_y <= 240.0, "{label} cut off at y={max_y}");
    }
}

#[test]
fn field_labels_show_help_on_hover() {
    // Arrange
    let mut f = ConnectionForm::new(DriverKind::Postgres, false);
    f.set_ssh(false);
    let mut h = Harness::builder()
        .with_accessibility_check(false) // text inputs and spin buttons are unnamed (#186)
        .build_ui(|ui| {
            show(ui, &mut f, &[]);
        });
    h.ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
    h.run();

    for (label, help) in [
        ("TLS mode", "VerifyFull: VerifyCa plus"),
        ("CA file", "PEM file with the CA certificate"),
        ("Page size", "Rows fetched per page"),
        ("Editor row limit", "Rows fetched per batch"),
        ("Safety threshold", "require password confirmation"),
        ("SSH tunnel", "Connect through an SSH server"),
    ] {
        // Act
        // Only the labelled combo matches "TLS mode" (no separate label node),
        // so hover the label cell by position instead of the node.
        h.get_all_by_label(label).last().unwrap().scroll_to_me();
        h.run();
        let r = h.get_all_by_label(label).last().unwrap().rect();
        h.hover_at(egui::pos2(12.0, r.center().y));
        h.run();

        // Assert
        h.get_by_label_contains(help);
    }
}

#[test]
fn label_picker_selects_one_or_none() {
    // Arrange
    let cat = ["dev", "prod"].map(|n| Label {
        name: n.into(),
        color: [1, 2, 3],
    });
    let mut f = ConnectionForm::new(DriverKind::Postgres, false);
    f.draft.label = Some("dev".into());
    let mut h = Harness::builder()
        .with_accessibility_check(false) // text inputs and spin buttons are unnamed (#186)
        .build_ui(|ui| {
            show(ui, &mut f, &cat);
        });
    h.run();

    // Act
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Label")
        .scroll_to_me();
    h.run();
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Label")
        .click();
    h.run();
    h.get_by_label("None").click();
    h.run();
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Label")
        .scroll_to_me();
    h.run();
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Label")
        .click();
    h.run();
    h.get_by_label("prod").click();
    h.run();
    drop(h);

    // Assert
    assert_eq!(f.draft.label.as_deref(), Some("prod"));
}

#[test]
fn combos_have_accessible_names() {
    // Arrange
    let mut f = ConnectionForm::new(DriverKind::Postgres, false);
    f.set_ssh(true);
    let mut h = Harness::builder()
        .with_accessibility_check(false) // text inputs and spin buttons are unnamed (#186)
        .build_ui(|ui| {
            show(ui, &mut f, &[]);
        });

    // Act
    h.run();

    // Assert
    for name in ["TLS mode", "Driver", "Auth"] {
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, name);
    }
}
