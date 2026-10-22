use crustdb_core::format::{hex_dump, pretty_json};
use crustdb_core::model::{Cell, ColumnMeta, Row, TypeCategory};
use eframe::egui::{self, RichText};

const HEX_BYTES: usize = 4096;
const TRUNCATED: &str = "… truncated at 1 MiB";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailLine {
    pub column: String,
    pub type_name: String,
    pub value: String,
}

pub fn row_detail_lines(columns: &[ColumnMeta], row: &Row) -> Vec<DetailLine> {
    columns
        .iter()
        .zip(row.iter())
        .map(|(c, cell)| {
            let value = match cell {
                Cell::Null => "NULL".to_string(),
                Cell::Text { value, truncated } => {
                    let mut v = match c.category {
                        TypeCategory::Json => pretty_json(value),
                        _ => None,
                    }
                    .unwrap_or_else(|| value.to_string());
                    if *truncated {
                        v.push('\n');
                        v.push_str(TRUNCATED);
                    }
                    v
                }
                Cell::Bytes {
                    value,
                    total_len,
                    truncated,
                } => {
                    let mut v = format!("{total_len} bytes\n{}", hex_dump(value, HEX_BYTES));
                    if *truncated {
                        v.push_str(TRUNCATED);
                    }
                    v
                }
            };
            DetailLine {
                column: c.name.clone(),
                type_name: c.type_name.clone(),
                value,
            }
        })
        .collect()
}

/// Nothing is selectable (D17: v1 has no cell copy).
pub fn show(ui: &mut egui::Ui, columns: &[ColumnMeta], row: &Row) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        for l in row_detail_lines(columns, row) {
            ui.add(
                egui::Label::new(RichText::new(format!("{}  {}", l.column, l.type_name)).strong())
                    .selectable(false),
            );
            ui.add(egui::Label::new(RichText::new(l.value).monospace()).selectable(false));
            ui.separator();
        }
    });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn col(name: &str, category: TypeCategory) -> ColumnMeta {
        ColumnMeta {
            name: name.into(),
            type_name: "t".into(),
            category,
            nullable: None,
            is_primary_key: false,
        }
    }

    #[test]
    fn row_detail_pretty_prints_json_and_hex_dumps_binary() {
        let cols = [col("j", TypeCategory::Json), col("b", TypeCategory::Binary)];
        let row: Row = vec![
            Cell::text_capped(r#"{"k":1}"#),
            Cell::bytes_capped(&[0x41; 5000]),
        ]
        .into();
        let l = row_detail_lines(&cols, &row);
        assert_eq!(l[0].value, "{\n  \"k\": 1\n}");
        assert_eq!((l[0].column.as_str(), l[0].type_name.as_str()), ("j", "t"));
        assert!(l[1].value.starts_with("5000 bytes\n00000000  41 41 "));
    }

    #[test]
    fn row_detail_marks_truncated() {
        let cols = [col("a", TypeCategory::Text), col("b", TypeCategory::Binary)];
        let n = crustdb_core::model::CELL_MEMORY_CAP + 1;
        let row: Row = vec![
            Cell::text_capped(&"a".repeat(n)),
            Cell::bytes_capped(&vec![0; n]),
        ]
        .into();
        let l = row_detail_lines(&cols, &row);
        assert!(l[0].value.ends_with("truncated at 1 MiB"));
        assert!(l[1].value.ends_with("truncated at 1 MiB"));
        assert_eq!(l[0].value.len(), 1_048_576 + "\n… truncated at 1 MiB".len());
    }
}
