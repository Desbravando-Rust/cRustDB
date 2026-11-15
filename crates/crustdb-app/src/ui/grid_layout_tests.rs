#![allow(clippy::unwrap_used, clippy::expect_used)]
use crustdb_core::model::{Cell, ColumnMeta, Row, TypeCategory};
use eframe::egui;
use egui_kittest::Harness;

use super::grid;
use crate::ui::grid_layout::{bool_text, column_widths, layout, next_epoch};
use egui_kittest::kittest::Queryable;

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
    assert_eq!(bool_text(&pg, "t"), Some("true"));
    assert_eq!(bool_text(&pg, "FALSE"), Some("false"));
    assert_eq!(bool_text(&my, "1"), Some("true"));
    assert_eq!(bool_text(&my, "0"), Some("false"));
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

#[test]
fn width_counts_wide_glyphs_as_two_columns() {
    // Arrange
    let cols = [meta("t", "text", TypeCategory::Text)];
    let rows = [text_row(&["日本語日本語"]), text_row(&["🎉🎉🎉🎉🎉🎉"])];

    // Act
    let w = column_widths(&cols, &rows, None, 7.0, 40.0);

    // Assert
    assert!(
        w[0] >= 12.0 * 7.0,
        "width {} too narrow for 12 columns",
        w[0]
    );
}

#[test]
fn width_is_capped_for_long_headers_too() {
    // Arrange
    let cols = [meta(&"h".repeat(300), "text", TypeCategory::Text)];

    // Act
    let w = column_widths(&cols, &[], None, 7.0, 60.0);

    // Assert
    assert!(w[0] <= 480.0);
}

#[test]
fn epoch_survives_more_rows_but_resets_on_new_columns_or_first_rows() {
    // Act & Assert
    assert_eq!(next_epoch(None, 1, true), (0, true));
    assert_eq!(
        next_epoch(Some((1, 0, true)), 1, true),
        (0, false),
        "10 -> 20 rows"
    );
    assert_eq!(
        next_epoch(Some((1, 0, true)), 1, false),
        (0, false),
        "refresh clears rows"
    );
    assert_eq!(
        next_epoch(Some((1, 0, false)), 1, true),
        (1, true),
        "0 -> rows"
    );
    assert_eq!(
        next_epoch(Some((1, 3, true)), 2, true),
        (4, true),
        "new columns"
    );
}

fn timestamp_harness(rows: Vec<Row>, cols: Vec<ColumnMeta>) -> Harness<'static> {
    let (mut selected, mut anchor) = (None, None);
    Harness::builder()
        .with_size(egui::vec2(800.0, 300.0))
        .build_ui(move |ui| {
            crate::ui::theme::install_theme(ui.ctx());
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
        })
}

#[test]
fn timestamp_cell_is_not_narrower_than_its_text() {
    // Arrange
    let text = "2024-01-01 12:00:00.123456";
    let cols = vec![meta("created_at", "timestamp", TypeCategory::Timestamp)];
    let mut h = timestamp_harness(vec![text_row(&[text])], cols);

    // Act
    h.run();
    h.run();

    // Assert
    let font = egui::TextStyle::Monospace.resolve(&h.ctx.global_style());
    let galley = h.ctx.fonts_mut(|f| {
        f.layout_no_wrap(text.into(), font, egui::Color32::WHITE)
            .size()
            .x
    });
    let width = h.get_by_label(text).rect().width();
    assert!(width >= galley - 0.5, "cell {width}px < text {galley}px");
}

#[test]
fn tinyint1_cell_is_green_when_true() {
    // Arrange
    let cols = [meta("flag", "tinyint(1)", TypeCategory::Integer)];
    let rows = vec![text_row(&["1"])];
    let (mut selected, mut anchor) = (None, None);
    let color = std::cell::Cell::new(None);
    let mut h = Harness::builder()
        .with_size(egui::vec2(400.0, 200.0))
        .build_ui(|ui| {
            crate::ui::theme::install_theme(ui.ctx());
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
            ui.ctx().graphics(|g| {
                for s in g
                    .get(ui.layer_id())
                    .into_iter()
                    .flat_map(|l| l.all_entries())
                {
                    if let egui::Shape::Text(t) = &s.shape
                        && t.galley.text() == "true"
                    {
                        color.set(t.galley.job.sections.first().map(|x| x.format.color));
                    }
                }
            });
        });

    // Act
    h.run();

    // Assert
    assert_eq!(color.get(), Some(egui::Color32::from_rgb(90, 190, 110)));
}

/// State id `layout` returns for `cols`/`rows` under `tab`, per scenario step.
fn salts(steps: &[(u64, &[ColumnMeta], &[Row])]) -> Vec<egui::Id> {
    let step = std::cell::Cell::new(0);
    let out = std::cell::RefCell::new(vec![egui::Id::NULL; steps.len()]);
    let mut h = Harness::builder().build_ui(|ui| {
        let (tab, cols, rows) = steps[step.get()];
        let (salt, _) = ui.push_id(tab, |ui| layout(ui, cols, rows, None)).inner;
        out.borrow_mut()[step.get()] = salt;
    });
    for i in 0..steps.len() {
        step.set(i);
        h.run();
    }
    drop(h);
    out.into_inner()
}

#[test]
fn switching_between_tabs_keeps_each_tabs_state_id() {
    // Arrange
    let a = [meta("a", "text", TypeCategory::Text)];
    let b = [meta("b", "int", TypeCategory::Integer)];
    let rows = [text_row(&["x"])];

    // Act
    let s = salts(&[(1, &a, &rows), (2, &b, &rows), (1, &a, &rows)]);

    // Assert
    assert_eq!(s[0], s[2]);
    assert_ne!(s[0], s[1]);
}

#[test]
fn refresh_showing_zero_rows_keeps_state_id() {
    // Arrange
    let a = [meta("a", "text", TypeCategory::Text)];
    let rows = [text_row(&["x"])];

    // Act
    let s = salts(&[(1, &a, &rows), (1, &a, &[]), (1, &a, &rows)]);

    // Assert
    assert_eq!(s[0], s[2]);
}
