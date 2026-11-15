//! Grid sizing and display helpers.
use crustdb_core::format::display_cell;
use crustdb_core::model::{ColumnMeta, GRID_DISPLAY_CHARS, Row, TypeCategory};

/// Cells and headers are padded by the table's item spacing; keep a little slack.
const PAD: f32 = 16.0;
/// Upper bound so one huge text column does not push the rest off screen.
const MAX_W: f32 = 480.0;
/// Rows sampled per column; enough to see timestamps and typical text.
const SAMPLE_ROWS: usize = 100;

pub fn column_widths(
    columns: &[ColumnMeta],
    rows: &[Row],
    view: Option<&[usize]>,
    char_w: f32,
    min_w: f32,
) -> Vec<f32> {
    let n = view.map_or(rows.len(), <[_]>::len).min(SAMPLE_ROWS);
    (0..columns.len())
        .map(|j| {
            let header = (columns[j].name.chars().count() + columns[j].type_name.chars().count())
                as f32
                * char_w
                + 24.0;
            let chars = (0..n)
                .filter_map(|k| rows.get(view.map_or(k, |v| v[k]))?.get(j))
                .map(|c| {
                    display_cell(c, &columns[j], GRID_DISPLAY_CHARS)
                        .chars()
                        .count()
                })
                .max()
                .unwrap_or(0);
            (chars as f32 * char_w + PAD)
                .clamp(0.0, MAX_W)
                .max(header)
                .max(min_w)
        })
        .collect()
}

/// `true`/`false` for boolean-like columns (`bool`, MySQL `tinyint(1)`); display only,
/// copy, export and edit keep the stored value. `None` when `text` is not a boolean literal.
pub fn bool_text(meta: &ColumnMeta, text: &str) -> Option<String> {
    let is_bool =
        meta.category == TypeCategory::Bool || meta.type_name.eq_ignore_ascii_case("tinyint(1)");
    if !is_bool {
        return None;
    }
    match text.to_lowercase().as_str() {
        "true" | "t" | "1" => Some("true".into()),
        "false" | "f" | "0" => Some("false".into()),
        _ => None,
    }
}
