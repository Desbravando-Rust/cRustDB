//! Read-only ER diagram of one schema: grid layout of table boxes and FK edges.
use eframe::egui::Rect;

use crate::runtime::DiagramTable;

/// Box position of each table, in `tables` order, relative to the diagram origin.
pub fn layout(tables: &[DiagramTable]) -> Vec<Rect> {
    let _ = tables;
    vec![]
}

/// `(from, to)` indices of each FK whose target table is in `tables`; others are skipped.
pub fn edges(tables: &[DiagramTable]) -> Vec<(usize, usize)> {
    let _ = tables;
    vec![]
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crustdb_core::model::{ColumnMeta, ForeignKeyInfo, ObjectKind, ObjectRef, TypeCategory};

    use super::*;

    fn obj(name: &str) -> ObjectRef {
        ObjectRef {
            database: "d".into(),
            schema: Some("public".into()),
            name: name.into(),
            kind: ObjectKind::Table,
        }
    }

    fn table(name: &str, cols: usize, fks: &[&str]) -> DiagramTable {
        DiagramTable {
            object: obj(name),
            columns: (0..cols)
                .map(|i| ColumnMeta {
                    name: format!("c{i}"),
                    type_name: "int4".into(),
                    category: TypeCategory::Integer,
                    nullable: Some(false),
                    is_primary_key: i == 0,
                })
                .collect(),
            fks: fks
                .iter()
                .map(|t| ForeignKeyInfo {
                    name: format!("fk_{t}"),
                    columns: vec!["c1".into()],
                    ref_table: obj(t),
                    ref_columns: vec!["c0".into()],
                    on_update: "NO ACTION".into(),
                    on_delete: "NO ACTION".into(),
                })
                .collect(),
        }
    }

    #[test]
    fn layout_places_tables_in_grid_without_overlap() {
        let tables: Vec<_> = (1..=5).map(|n| table(&format!("t{n}"), n, &[])).collect();
        let rects = layout(&tables);
        assert_eq!(rects.len(), 5);
        // ceil(sqrt(5)) = 3 columns: t4 starts a second row under t1.
        assert!(rects[1].min.x > rects[0].max.x);
        assert!(rects[2].min.x > rects[1].max.x);
        assert_eq!(rects[3].min.x, rects[0].min.x);
        assert!(rects[3].min.y > rects[2].max.y);
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                assert!(!a.intersects(*b), "{a:?} overlaps {b:?}");
            }
        }
        // Height grows with the column count.
        assert!(rects[4].height() > rects[3].height());
        assert!(rects[1].height() > rects[0].height());
    }

    #[test]
    fn edges_skip_fks_to_tables_outside_schema() {
        let tables = [
            table("customers", 2, &[]),
            table("orders", 3, &["customers", "elsewhere"]),
        ];
        assert_eq!(edges(&tables), [(1, 0)]);
        assert!(edges(&[]).is_empty());
    }
}
