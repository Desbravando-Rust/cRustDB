use crustdb_core::format::display_cell;
use crustdb_core::model::{Cell, ColumnMeta, GRID_DISPLAY_CHARS, Row};
use crustdb_core::query::Sort;
use crustdb_core::sql::{TokenKind, highlight};
use eframe::egui::{self, Color32, RichText};
use egui_extras::{Column, TableBuilder};

use super::{Intent, grid_copy};
use crate::state::{TableTab, TableView};

const ROW_H: f32 = 20.0;

fn sort_mark(sorts: &[Sort], column: &str) -> String {
    match sorts.iter().position(|s| s.column == column) {
        Some(i) => {
            let arrow = if sorts[i].descending { "▼" } else { "▲" };
            if sorts.len() > 1 {
                format!(" {arrow}{}", i + 1)
            } else {
                format!(" {arrow}")
            }
        }
        None => String::new(),
    }
}

/// What the inline cell editor asks the caller to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellOp {
    Begin { row: usize, col: usize },
    Commit,
    Cancel,
    SetNull { row: usize, col: usize },
}

/// The cell being edited (`rows` index, column, text) and the op raised this frame.
pub type CellEdit<'a> = (
    &'a mut Option<(usize, usize, String)>,
    &'a mut Option<CellOp>,
);

/// Virtualized grid over `rows` (or the `view` indexes into it); `selected` (cursor) and
/// `anchor` (other end of a Shift+click range) are `rows` indexes. The range spans display
/// positions. `edit` enables inline editing (double click, Enter commits, Esc cancels).
/// Returns the clicked header as `(column, shift)`.
#[allow(clippy::too_many_arguments)]
pub fn grid(
    ui: &mut egui::Ui,
    columns: &[ColumnMeta],
    rows: &[Row],
    view: Option<&[usize]>,
    selected: &mut Option<usize>,
    anchor: &mut Option<usize>,
    sorts: &[Sort],
    mut edit: Option<CellEdit<'_>>,
) -> Option<(String, bool)> {
    let mut clicked = None;
    // ponytail: O(n) scan of `view` per frame; keep positions in state if big views lag.
    let pos = |i: usize| view.map_or(Some(i), |v| v.iter().position(|&x| x == i));
    let sel = grid_copy::range(selected.and_then(pos), anchor.and_then(pos));
    grid_copy::shortcut(ui, columns, rows, view, sel.clone());
    let mut table = TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .sense(egui::Sense::click());
    for _ in columns {
        table = table.column(Column::auto().at_least(60.0).clip(true));
    }
    table
        .header(ROW_H, |mut h| {
            for c in columns {
                h.col(|ui| {
                    let text = format!("{}{}", c.name, sort_mark(sorts, &c.name));
                    let r = ui.add(
                        egui::Label::new(RichText::new(text).strong())
                            .selectable(false)
                            .sense(egui::Sense::click()),
                    );
                    if r.clicked() {
                        clicked = Some((c.name.clone(), ui.input(|i| i.modifiers.shift)));
                    }
                });
            }
        })
        .body(|body| {
            let n = view.map_or(rows.len(), <[_]>::len);
            body.rows(ROW_H, n, |mut row| {
                let k = row.index();
                let i = view.and_then(|v| v.get(k).copied()).unwrap_or(k);
                row.set_selected(sel.as_ref().is_some_and(|r| r.contains(&k)));
                let Some(cells) = rows.get(i) else { return };
                for (j, (cell, meta)) in cells.iter().zip(columns).enumerate() {
                    // Labels are not selectable so clicks reach the cell (selection, context menu).
                    let (_, cell_resp) = row.col(|ui| {
                        if let Some((Some((r, c, text)), op)) = edit.as_mut()
                            && (*r, *c) == (i, j)
                        {
                            return edit_cell(ui, text, op);
                        }
                        let text = display_cell(cell, meta, GRID_DISPLAY_CHARS);
                        if matches!(cell, Cell::Null) {
                            let null = RichText::new(text).italics().color(Color32::GRAY);
                            ui.add(egui::Label::new(null).selectable(false));
                        } else {
                            ui.add(egui::Label::new(text).truncate().selectable(false));
                        }
                    });
                    cell_resp.context_menu(|ui| {
                        grid_copy::menu(ui, columns, rows, view, sel.clone(), k, cell);
                        if let Some((_, op)) = edit.as_mut()
                            && ui.button("Set NULL").clicked()
                        {
                            **op = Some(CellOp::SetNull { row: i, col: j });
                        }
                    });
                    if let Some((_, op)) = edit.as_mut()
                        && cell_resp.double_clicked()
                    {
                        **op = Some(CellOp::Begin { row: i, col: j });
                    }
                }
                let resp = row.response();
                if resp.clicked() {
                    if !(resp.ctx.input(|inp| inp.modifiers.shift) && selected.is_some()) {
                        *anchor = Some(i);
                    }
                    *selected = Some(i);
                }
            });
        });
    clicked
}

/// Focused single-line editor: Enter asks to commit, Esc or clicking away cancels.
fn edit_cell(ui: &mut egui::Ui, text: &mut String, op: &mut Option<CellOp>) {
    let r = ui.add(
        egui::TextEdit::singleline(text)
            .hint_text("Cell value")
            .desired_width(f32::INFINITY),
    );
    if r.lost_focus() {
        *op = Some(if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            // egui 0.37 reports `lost_focus` for two frames; refocus now so
            // the next frame does not read it as a click-away cancel.
            r.request_focus();
            CellOp::Commit
        } else {
            CellOp::Cancel
        });
    } else if !r.has_focus() {
        r.request_focus();
    }
}

pub fn show(ui: &mut egui::Ui, tab: &mut TableTab) -> Vec<Intent> {
    let mut out = vec![];
    let id = tab.id;
    egui::Panel::bottom(("table_footer", id.0)).show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(tab.page > 0, egui::Button::new("<"))
                .clicked()
            {
                out.push(Intent::PrevPage(id));
            }
            if ui
                .add_enabled(tab.has_more, egui::Button::new(">"))
                .clicked()
            {
                out.push(Intent::NextPage(id));
            }
            ui.label(tab.footer_label());
            if tab.is_loading() && ui.button("Stop").clicked() {
                out.push(Intent::StopTab(id));
            }
            if let Some(n) = tab.notice.clone() {
                ui.colored_label(ui.visuals().error_fg_color, n);
                if ui.button("Retry").clicked() {
                    out.push(Intent::Retry(id));
                }
            }
            if tab.unordered() {
                ui.colored_label(Color32::YELLOW, "unordered");
            }
        });
    });
    if tab.filter_open {
        egui::Panel::top(("filter_bar", id.0)).show(ui, |ui| {
            out.extend(super::filter_bar::show(ui, tab));
        });
    }
    if tab.detail_open
        && let Some(i) = tab.selected_row
        && let Some(row) = tab.rows.get(i)
    {
        egui::Panel::right(("row_detail", id.0)).show(ui, |ui| {
            super::row_detail::show(ui, &tab.columns, row);
        });
    }
    egui::CentralPanel::default().show(ui, |ui| {
        let mut op = None;
        let clicked = grid(
            ui,
            &tab.columns,
            &tab.rows,
            None,
            &mut tab.selected_row,
            &mut tab.select_anchor,
            &tab.sorts,
            Some((&mut tab.editing, &mut op)),
        );
        match op {
            Some(CellOp::Begin { row, col }) => tab.begin_edit(row, col),
            Some(CellOp::Commit) => out.push(Intent::CommitEdit(id)),
            Some(CellOp::Cancel) => tab.editing = None,
            Some(CellOp::SetNull { row, col }) => out.push(Intent::SetNull { tab: id, row, col }),
            None => {}
        }
        if let Some((column, shift)) = clicked {
            out.push(Intent::SortHeader {
                tab: id,
                column,
                shift,
            });
        }
    });
    out
}

/// DDL view: loading, error + Retry, or read-only highlighted text.
fn show_ddl(ui: &mut egui::Ui, tab: &TableTab) -> Vec<Intent> {
    let mut out = vec![];
    if let Some(n) = &tab.notice {
        ui.colored_label(Color32::LIGHT_RED, n);
        if ui.button("Retry").clicked() {
            out.push(Intent::Retry(tab.id));
        }
    } else if let Some(t) = &tab.ddl {
        let mut job = egui::text::LayoutJob::default();
        for (r, k) in highlight(t, tab.driver) {
            let color = match k {
                TokenKind::Keyword => Color32::from_rgb(200, 120, 220),
                TokenKind::String => Color32::from_rgb(150, 200, 120),
                TokenKind::Number => Color32::from_rgb(230, 170, 90),
                TokenKind::Comment => Color32::GRAY,
                TokenKind::QuotedIdent => Color32::from_rgb(120, 190, 230),
                _ => Color32::LIGHT_GRAY,
            };
            job.append(
                &t[r],
                0.0,
                egui::TextFormat {
                    font_id: egui::FontId::monospace(13.0),
                    color,
                    ..Default::default()
                },
            );
        }
        egui::ScrollArea::both().show(ui, |ui| ui.add(egui::Label::new(job).selectable(true)));
    } else {
        ui.label("Loading DDL…");
    }
    out
}

/// Tab body: view switcher, then the Data / Structure / DDL view.
pub fn show_tab(ui: &mut egui::Ui, tab: &mut TableTab) -> Vec<Intent> {
    let mut out = vec![];
    let is_table = tab.view != TableView::Ddl || tab.structure.is_some();
    if is_table {
        ui.horizontal(|ui| {
            for (v, label) in [
                (TableView::Data, "Data"),
                (TableView::Structure, "Structure"),
                (TableView::Ddl, "DDL"),
            ] {
                if ui.selectable_label(tab.view == v, label).clicked() && tab.view != v {
                    out.push(Intent::SetView(tab.id, v));
                }
            }
        });
    }
    match tab.view {
        TableView::Data => out.extend(show(ui, tab)),
        TableView::Structure => out.extend(super::structure::show(ui, tab)),
        TableView::Ddl => out.extend(show_ddl(ui, tab)),
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crustdb_core::model::{ObjectKind, ObjectRef, TypeCategory};
    use egui_kittest::Harness;

    use super::*;
    use crate::runtime::TabId;

    fn tab() -> TableTab {
        let object = ObjectRef {
            database: "d".into(),
            schema: None,
            name: "t".into(),
            kind: ObjectKind::Table,
        };
        let mut t = TableTab::new(TabId(1), object, 300);
        t.columns = ["id", "v"]
            .iter()
            .map(|n| ColumnMeta {
                name: (*n).into(),
                type_name: "text".into(),
                category: TypeCategory::Text,
                nullable: None,
                is_primary_key: *n == "id",
            })
            .collect();
        t.rows = vec![vec![Cell::text_capped("1"), Cell::text_capped("old")].into()];
        t.editing = Some((0, 1, "old".into()));
        t
    }

    fn run_keys(t: &mut TableTab, text: &str, key: egui::Key) -> Vec<Intent> {
        let mut out = vec![];
        let mut h = Harness::new_ui(|ui| out.extend(show(ui, t)));
        h.run();
        h.event(egui::Event::Text(text.into()));
        h.run();
        h.key_press(key);
        h.run();
        drop(h);
        out
    }

    #[test]
    fn enter_in_cell_editor_commits_typed_text() {
        let mut t = tab();
        let out = run_keys(&mut t, "x", egui::Key::Enter);
        assert!(
            out.iter()
                .any(|i| matches!(i, Intent::CommitEdit(TabId(1))))
        );
        assert!(t.editing.unwrap().2.contains('x'));
    }

    #[test]
    fn escape_in_cell_editor_cancels() {
        let mut t = tab();
        let out = run_keys(&mut t, "x", egui::Key::Escape);
        assert!(out.is_empty());
        assert!(t.editing.is_none());
    }
}
