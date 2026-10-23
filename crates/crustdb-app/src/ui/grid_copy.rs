//! Clipboard copy for the result grid: the platform Copy event (Ctrl+C) and the cell context menu.
use std::ops::RangeInclusive;

use crustdb_core::model::{Cell, ColumnMeta, Row};
use eframe::egui;

use crate::csv_out::{cell_text, to_tsv};

/// Selected rows between the cursor and the Shift+click anchor.
pub fn range(cursor: Option<usize>, anchor: Option<usize>) -> Option<RangeInclusive<usize>> {
    let c = cursor?;
    let a = anchor.unwrap_or(c);
    Some(c.min(a)..=c.max(a))
}

/// Rows at display positions `r`, mapped through `view` when the grid is sorted/filtered.
fn pick<'a>(
    rows: &'a [Row],
    view: Option<&'a [usize]>,
    r: RangeInclusive<usize>,
) -> impl Iterator<Item = &'a Row> {
    r.filter_map(move |k| rows.get(view.map_or(Some(k), |v| v.get(k).copied())?))
}

/// Copies the selected rows as TSV on the Copy event, unless a widget (e.g. the editor) has focus.
pub fn shortcut(
    ui: &egui::Ui,
    columns: &[ColumnMeta],
    rows: &[Row],
    view: Option<&[usize]>,
    sel: Option<RangeInclusive<usize>>,
) {
    let copy = ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy)))
        && ui.memory(|m| m.focused().is_none());
    if copy && let Some(r) = sel {
        ui.ctx()
            .copy_text(to_tsv(columns, pick(rows, view, r), false));
    }
}

/// Menu of a cell at display position `row`; row copies use the selection if it holds `row`,
/// else that row.
pub fn menu(
    ui: &mut egui::Ui,
    columns: &[ColumnMeta],
    rows: &[Row],
    view: Option<&[usize]>,
    sel: Option<RangeInclusive<usize>>,
    row: usize,
    cell: &Cell,
) {
    let r = sel.filter(|r| r.contains(&row)).unwrap_or(row..=row);
    if ui.button("Copy cell").clicked() {
        ui.ctx().copy_text(cell_text(cell));
    }
    if ui.button("Copy row(s)").clicked() {
        ui.ctx()
            .copy_text(to_tsv(columns, pick(rows, view, r.clone()), false));
    }
    if ui.button("Copy row(s) with header").clicked() {
        ui.ctx()
            .copy_text(to_tsv(columns, pick(rows, view, r), true));
    }
}
