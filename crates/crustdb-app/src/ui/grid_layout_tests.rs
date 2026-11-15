#![allow(clippy::unwrap_used, clippy::expect_used)]
use crustdb_core::model::{Cell, ColumnMeta, Row, TypeCategory};
use eframe::egui;
use egui_kittest::Harness;

use super::grid;
use crate::ui::grid_layout::{bool_text, column_widths};

fn meta(name: &str, type_name: &str, category: TypeCategory) -> ColumnMeta {
    ColumnMeta {
        name: name.into(),
        type_name: type_name.into(),
        category,
        nullable: None,
        is_primary_key: false,
    }
}

fn text_row(values: &[&str]) -> Row {
    values.iter().map(|v| Cell::text_capped(v)).collect()
}

#[test]
fn width_fits_full_timestamp() {
    // Arrange
    let cols = [meta("created_at", "timestamp", TypeCategory::Timestamp)];
    let rows = [text_row(&["2024-01-01 12:00:00.123456"])];

    // Act
    let w = column_widths(&cols, &rows, None, 7.0, 40.0);

    // Assert
    assert!(w[0] >= 26.0 * 7.0, "width {} cuts the timestamp", w[0]);
}

#[test]
fn width_never_below_header_or_minimum() {
    // Arrange
    let cols = [meta("a", "int", TypeCategory::Integer)];
    let rows = [text_row(&["1"])];

    // Act
    let w = column_widths(&cols, &rows, None, 7.0, 90.0);

    // Assert
    assert!(w[0] >= 90.0);
}

#[test]
fn width_of_long_text_is_capped() {
    // Arrange
    let cols = [meta("t", "text", TypeCategory::Text)];
    let long = "x".repeat(200);
    let rows = [text_row(&[&long])];

    // Act
    let w = column_widths(&cols, &rows, None, 7.0, 40.0);

    // Assert
    assert!(w[0] <= 500.0);
}

#[test]
fn bool_columns_display_true_false() {
    // Arrange
    let pg = meta("b", "bool", TypeCategory::Bool);
    let my = meta("b", "tinyint(1)", TypeCategory::Integer);
    let int = meta("n", "tinyint(4)", TypeCategory::Integer);

    // Act & Assert
    assert_eq!(bool_text(&pg, "t").as_deref(), Some("true"));
    assert_eq!(bool_text(&pg, "false").as_deref(), Some("false"));
    assert_eq!(bool_text(&my, "1").as_deref(), Some("true"));
    assert_eq!(bool_text(&my, "0").as_deref(), Some("false"));
    assert_eq!(bool_text(&my, "2"), None);
    assert_eq!(bool_text(&int, "1"), None);
}

/// Lowest y of the vertical line segments (column dividers) painted so far on this layer.
fn divider_bottom(ui: &egui::Ui) -> f32 {
    ui.ctx().graphics(|g| {
        g.get(ui.layer_id()).map_or(0.0, |list| {
            list.all_entries()
                .filter_map(|s| match &s.shape {
                    egui::Shape::LineSegment { points, .. } if points[0].x == points[1].x => {
                        Some(points[1].y)
                    }
                    _ => None,
                })
                .fold(0.0, f32::max)
        })
    })
}

#[test]
fn grid_does_not_fill_panel_with_dividers_when_few_rows() {
    // Arrange
    let cols = [meta("a", "text", TypeCategory::Text)];
    let rows = vec![text_row(&["x"]), text_row(&["y"])];
    let (mut selected, mut anchor) = (None, None);
    let height = std::cell::Cell::new(0.0_f32);
    let mut h = Harness::builder()
        .with_size(egui::vec2(800.0, 600.0))
        .build_ui(|ui| {
            crate::ui::theme::install_theme(ui.ctx());
            egui::CentralPanel::default().show(ui, |ui| {
                let top = ui.cursor().top();
                grid(
                    ui,
                    &cols,
                    &rows,
                    None,
                    &mut selected,
                    &mut anchor,
                    &[],
                    None,
                    &[],
                    None,
                );
                height.set(divider_bottom(ui) - top);
            });
        });

    // Act
    h.run();

    // Assert
    let height = height.get();
    assert!(height < 120.0, "dividers span {height}px for 2 rows");
}
