//! Grid sizing and display helpers.
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crustdb_core::model::{Cell, ColumnMeta, GRID_DISPLAY_CHARS, Row, TypeCategory};
use eframe::egui;
use unicode_width::UnicodeWidthChar;

/// Cells and headers are padded by the table's item spacing; keep a little slack.
const PAD: f32 = 16.0;
/// Upper bound so one huge column does not push the rest off screen (headers included).
const MAX_W: f32 = 480.0;
const MIN_W: f32 = 60.0;
/// Rows sampled per column. ponytail: values longer than the sample (seen after Load more
/// or scrolling) can still be clipped; widen by resizing the column.
const SAMPLE_ROWS: usize = 100;
/// `(binary 1023.9 KB) ` plus a 16-byte hex preview.
const BYTES_COLS: usize = 52;

fn cols_of(s: &str) -> usize {
    s.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// Terminal columns the cell needs; avoids formatting a string per cell.
fn cell_cols(cell: &Cell) -> usize {
    match cell {
        Cell::Null => 4,
        Cell::Bytes { .. } => BYTES_COLS,
        Cell::Text { value, truncated } => {
            let mut n = 0;
            for (i, c) in value.chars().enumerate() {
                if i == GRID_DISPLAY_CHARS {
                    return n + 1; // "…"
                }
                n += c.width().unwrap_or(0);
            }
            n + usize::from(*truncated)
        }
    }
}

pub fn column_widths(
    columns: &[ColumnMeta],
    rows: &[Row],
    view: Option<&[usize]>,
    char_w: f32,
    min_w: f32,
) -> Vec<f32> {
    let n = view.map_or(rows.len(), <[_]>::len).min(SAMPLE_ROWS);
    columns
        .iter()
        .enumerate()
        .map(|(j, meta)| {
            let header = (cols_of(&meta.name) + cols_of(&meta.type_name)) as f32 * char_w + 24.0;
            let floor = if is_bool_col(meta) { 5 } else { 0 }; // "false"
            let chars = (0..n)
                .filter_map(|k| rows.get(view.map_or(k, |v| v[k]))?.get(j))
                .map(cell_cols)
                .max()
                .unwrap_or(0)
                .max(floor);
            (chars as f32 * char_w + PAD)
                .max(header)
                .clamp(min_w, MAX_W)
        })
        .collect()
}

#[derive(Clone)]
struct Cached {
    sig: u64,
    epoch: u64,
    /// Rows were seen under this signature; a refresh that shows 0 rows first keeps it.
    had_rows: bool,
    widths: Arc<[f32]>,
}

/// Hash of every column's name and type.
fn signature(columns: &[ColumnMeta]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for c in columns {
        (&c.name, &c.type_name).hash(&mut h);
    }
    h.finish()
}

/// Epoch of the table's saved state (widths, scroll): it changes, resetting both, only when
/// the columns change or the grid goes from empty to filled. Load more, refresh and a
/// provisional row keep it. Returns `(epoch, recompute_widths)`.
pub fn next_epoch(prev: Option<(u64, u64, bool)>, sig: u64, has_rows: bool) -> (u64, bool) {
    match prev {
        None => (0, true),
        Some((s, e, had)) if s != sig || (!had && has_rows) => (e + 1, true),
        Some((_, e, _)) => (e, false),
    }
}

/// Table state id and initial column widths. One cache entry per `ui.scope_id()`, so callers
/// showing several grids must push a distinct id per grid (`show` uses the tab id).
pub fn layout(
    ui: &mut egui::Ui,
    columns: &[ColumnMeta],
    rows: &[Row],
    view: Option<&[usize]>,
) -> (egui::Id, Arc<[f32]>) {
    let key = ui.scope_id().with("grid_layout");
    let prev: Option<Cached> = ui.data(|d| d.get_temp(key));
    let sig = signature(columns);
    let (epoch, recompute) = next_epoch(
        prev.as_ref().map(|c| (c.sig, c.epoch, c.had_rows)),
        sig,
        !rows.is_empty(),
    );
    let kept = !recompute && prev.as_ref().is_some_and(|c| c.had_rows);
    let had_rows = kept || !rows.is_empty();
    let widths = match prev {
        Some(c) if !recompute => c.widths,
        _ => {
            let font = egui::TextStyle::Monospace.resolve(ui.style());
            let char_w = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
            column_widths(columns, rows, view, char_w, MIN_W).into()
        }
    };
    ui.data_mut(|d| {
        d.insert_temp(
            key,
            Cached {
                sig,
                epoch,
                had_rows,
                widths: widths.clone(),
            },
        );
    });
    (egui::Id::unique(("grid", ui.scope_id(), epoch)), widths)
}

fn is_bool_col(meta: &ColumnMeta) -> bool {
    meta.category == TypeCategory::Bool || meta.type_name.eq_ignore_ascii_case("tinyint(1)")
}

/// `"true"`/`"false"` for a boolean literal in a boolean-like column (`bool`, MySQL
/// `tinyint(1)`), else `None`. Display only: copy, export, the cell editor (opens with
/// the stored `t`/`1`) and pending edits (shown raw) keep the stored value.
/// Limitation: raw MySQL queries report `tiny`, so only table browsing sees `tinyint(1)`.
pub fn bool_text(meta: &ColumnMeta, text: &str) -> Option<&'static str> {
    if !is_bool_col(meta) {
        return None;
    }
    let is = |a: &[&str]| a.iter().any(|v| text.eq_ignore_ascii_case(v));
    if is(&["true", "t", "1"]) {
        Some("true")
    } else if is(&["false", "f", "0"]) {
        Some("false")
    } else {
        None
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
