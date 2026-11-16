use crustdb_core::format::display_cell;
use crustdb_core::model::{Cell, ColumnMeta, GRID_DISPLAY_CHARS, ObjectKind, Row, TypeCategory};
use crustdb_core::query::Sort;
use crustdb_core::sql::{TokenKind, highlight};
use eframe::egui::{self, Color32, RichText};
use egui_extras::{Column, TableBuilder};

use super::grid_layout::{bool_text, layout};
use super::theme::ACCENT;
use super::{Intent, grid_copy};
use crate::state::{TableTab, TableView};

const ROW_H: f32 = 20.0;
/// Integers: light orange, legible on the dark panel and on the `ACCENT` selection fill.
pub(crate) const INT_COLOR: Color32 = Color32::from_rgb(254, 215, 170);
const GREEN: Color32 = Color32::from_rgb(90, 190, 110);
const AMBER: Color32 = Color32::from_rgb(230, 170, 60);

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
    Follow { row: usize, col: usize },
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
    edit: Option<CellEdit<'_>>,
    pending: &[(usize, Option<String>)],
    pending_row: Option<usize>,
) -> Option<(String, bool)> {
    // The table paints column dividers down to its parent's bottom edge; a child
    // ui only extends as far as the content, so the empty area stays clean.
    ui.scope(|ui| {
        table(
            ui,
            columns,
            rows,
            view,
            selected,
            anchor,
            sorts,
            edit,
            pending,
            pending_row,
        )
    })
    .inner
}

#[allow(clippy::too_many_arguments)]
fn table(
    ui: &mut egui::Ui,
    columns: &[ColumnMeta],
    rows: &[Row],
    view: Option<&[usize]>,
    selected: &mut Option<usize>,
    anchor: &mut Option<usize>,
    sorts: &[Sort],
    mut edit: Option<CellEdit<'_>>,
    pending: &[(usize, Option<String>)],
    pending_row: Option<usize>,
) -> Option<(String, bool)> {
    let mut clicked = None;
    // ponytail: O(n) scan of `view` per frame; keep positions in state if big views lag.
    let pos = |i: usize| view.map_or(Some(i), |v| v.iter().position(|&x| x == i));
    let sel = grid_copy::range(selected.and_then(pos), anchor.and_then(pos));
    grid_copy::shortcut(ui, columns, rows, view, sel.clone());
    // Size to content (dates and timestamps stay whole); see `grid_layout::layout`.
    let (salt, widths) = layout(ui, columns, rows, view);
    let mut table = TableBuilder::new(ui)
        .id_salt(salt)
        .striped(true)
        .resizable(true)
        .sense(egui::Sense::click());
    table = table.column(Column::auto().at_least(32.0));
    for &w in widths.iter() {
        table = table.column(Column::initial(w).at_least(60.0).clip(true));
    }
    table
        .header(ROW_H, |mut h| {
            h.col(|ui| {
                ui.label(RichText::new("#").weak());
            });
            for c in columns {
                h.col(|ui| {
                    let mark = sort_mark(sorts, &c.name);
                    let r = ui
                        .horizontal(|ui| {
                            let name = ui.add(
                                egui::Label::new(
                                    RichText::new(format!("{}{mark}", c.name))
                                        .strong()
                                        .size(14.0),
                                )
                                .selectable(false)
                                .sense(egui::Sense::click()),
                            );
                            // An unnamed type renders no tag.
                            match type_label(&c.type_name) {
                                Some(t) => {
                                    name | ui.add(
                                        egui::Label::new(RichText::new(t).small().weak())
                                            .selectable(false)
                                            .sense(egui::Sense::click()),
                                    )
                                }
                                None => name,
                            }
                        })
                        .inner;
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
                row.col(|ui| {
                    ui.add(
                        egui::Label::new(RichText::new((k + 1).to_string()).weak())
                            .selectable(false),
                    );
                });
                for (j, (cell, meta)) in cells.iter().zip(columns).enumerate() {
                    // Labels are not selectable so clicks reach the cell (selection, context menu).
                    let (_, cell_resp) = row.col(|ui| {
                        if let Some((Some((r, c, text)), op)) = edit.as_mut()
                            && (*r, *c) == (i, j)
                        {
                            return edit_cell(ui, text, op);
                        }
                        if pending_row == Some(i)
                            && let Some((_, v)) = pending.iter().find(|(c, _)| *c == j)
                        {
                            let warn = ui.visuals().warn_fg_color;
                            let t = RichText::new(v.as_deref().unwrap_or("NULL")).color(warn);
                            ui.add(egui::Label::new(t).truncate().selectable(false));
                            return;
                        }
                        let raw = display_cell(cell, meta, GRID_DISPLAY_CHARS);
                        // Display only: copy, export and edit keep the stored value.
                        let flag = bool_text(meta, &raw);
                        let text = flag.map_or(raw, std::borrow::Cow::Borrowed);
                        if matches!(cell, Cell::Null) {
                            let null = RichText::new(text).italics().color(Color32::GRAY);
                            ui.add(egui::Label::new(null).selectable(false));
                        } else {
                            let t = RichText::new(text).monospace();
                            let t = match (flag, meta.category) {
                                (Some("true"), _) => t.color(GREEN),
                                (Some(_), _) => t.color(Color32::GRAY),
                                (None, TypeCategory::Integer) => t.color(INT_COLOR),
                                _ => t,
                            };
                            ui.add(egui::Label::new(t).truncate().selectable(false));
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
                    if let Some((_, op)) = edit.as_mut()
                        && cell_resp.clicked()
                        && cell_resp.ctx.input(|inp| inp.modifiers.command)
                    {
                        **op = Some(CellOp::Follow { row: i, col: j });
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

/// Why the tab can never be edited from the grid; `None` while the columns are still unknown.
pub(super) fn read_only_reason(tab: &TableTab) -> Option<&'static str> {
    if tab.object.kind != ObjectKind::Table {
        Some("Views are read-only")
    } else if !tab.columns.is_empty() && !tab.columns.iter().any(|c| c.is_primary_key) {
        Some("Table has no primary key")
    } else {
        None
    }
}

/// Why "Add row"/"Delete rows" cannot act on the tab right now.
pub(super) fn write_block(tab: &TableTab) -> Option<&'static str> {
    if let Some(why) = read_only_reason(tab) {
        Some(why)
    } else if tab.has_pending() {
        Some("Save or discard pending changes")
    } else if tab.is_loading() || tab.edit_req.is_some() {
        Some("Wait for the current operation")
    } else {
        None
    }
}

fn delete_button(ui: &mut egui::Ui, tab: &TableTab, out: &mut Vec<Intent>) {
    let block = write_block(tab).or(tab
        .selected_row
        .is_none()
        .then_some("Select rows to delete"));
    let r = ui.add_enabled(block.is_none(), egui::Button::new("Delete rows"));
    let r = match block {
        Some(why) => r.on_disabled_hover_text(why),
        None => r,
    };
    if r.clicked() {
        out.push(Intent::DeleteRows(tab.id));
    }
}

/// Short uppercase type tag of a column header (`character varying(40)[]` -> `VARCHAR[]`).
/// Drops `(...)` arguments, keeps `[]` and `unsigned`, folds aliases; `None` for an empty name.
pub(crate) fn type_label(type_name: &str) -> Option<String> {
    let mut depth = 0usize;
    let bare: String = type_name
        .chars()
        .filter(|&c| match c {
            '(' => {
                depth += 1;
                false
            }
            ')' => {
                depth = depth.saturating_sub(1);
                false
            }
            _ => depth == 0,
        })
        .collect();
    let lower = bare.to_lowercase();
    let mut t = lower
        .replace(" without time zone", "")
        .replace(" with time zone", "tz");
    let mut arrays = String::new();
    while let Some(rest) = t.trim_end().strip_suffix("[]") {
        arrays.push_str("[]");
        t = rest.to_owned();
    }
    let mut words: Vec<&str> = t.split_whitespace().collect();
    let mut flags = vec![];
    while let Some(&w) = words
        .last()
        .filter(|w| matches!(**w, "unsigned" | "zerofill"))
    {
        flags.insert(0, w);
        words.pop();
    }
    let base = words.join(" ");
    let base = match base.as_str() {
        "character varying" => "varchar",
        "character" => "char",
        "integer" | "int4" => "int",
        "int2" => "smallint",
        "int8" => "bigint",
        "float4" => "real",
        "float8" | "double precision" => "double",
        "boolean" => "bool",
        b => b,
    };
    if base.is_empty() {
        return None;
    }
    let mut out = base.to_owned();
    for f in flags {
        out.push(' ');
        out.push_str(f);
    }
    out.push_str(&arrays);
    Some(out.to_uppercase())
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
            delete_button(ui, tab, &mut out);
            if tab.object.kind == ObjectKind::Table
                && let Some(why) = read_only_reason(tab)
            {
                ui.colored_label(
                    Color32::YELLOW,
                    format!(
                        "{} Read-only: {}",
                        egui_phosphor::regular::WARNING,
                        why.to_lowercase()
                    ),
                );
            }
            if tab.has_pending() {
                egui::Frame::new()
                    .stroke(egui::Stroke::new(1.0, ACCENT))
                    .corner_radius(4.0)
                    .inner_margin(4.0)
                    .show(ui, |ui| {
                        if ui.button("Discard").clicked() {
                            out.push(Intent::DiscardPending(id));
                        }
                        match tab.pending.len() {
                            0 => ui.label("New row"),
                            1 => ui.label("1 change pending"),
                            n => ui.label(format!("{n} changes pending")),
                        };
                    });
            }
            if let Some(n) = tab.notice.clone() {
                ui.colored_label(ui.visuals().error_fg_color, n);
            }
            if tab.unordered() {
                ui.colored_label(AMBER, "unordered");
            }
        });
    });
    out.extend(
        egui::Panel::top(("data_toolbar", id.0))
            .show(ui, |ui| super::data_toolbar::show(ui, tab))
            .inner,
    );
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
        // Per-tab id: widths and scroll state must not be shared between table tabs.
        let clicked = ui
            .push_id(id.0, |ui| {
                grid(
                    ui,
                    &tab.columns,
                    &tab.rows,
                    None,
                    &mut tab.selected_row,
                    &mut tab.select_anchor,
                    &tab.sorts,
                    Some((&mut tab.editing, &mut op)),
                    &tab.pending,
                    tab.pending_row,
                )
            })
            .inner;
        match op {
            Some(CellOp::Begin { row, col }) => tab.begin_edit(row, col),
            Some(CellOp::Commit) => out.push(Intent::CommitEdit(id)),
            Some(CellOp::Cancel) => tab.editing = None,
            Some(CellOp::SetNull { row, col }) => out.push(Intent::SetNull { tab: id, row, col }),
            Some(CellOp::Follow { row, col }) => out.push(Intent::FollowFk { tab: id, row, col }),
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
#[path = "grid_layout_tests.rs"]
mod layout_tests;
#[cfg(test)]
#[path = "grid_tests.rs"]
mod tests;
