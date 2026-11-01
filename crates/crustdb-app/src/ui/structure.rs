use eframe::egui;

use super::Intent;
use crate::state::TableTab;

pub fn show(ui: &mut egui::Ui, tab: &TableTab) -> Vec<Intent> {
    let mut out = vec![];
    let Some(s) = &tab.structure else {
        if let Some(n) = &tab.notice {
            ui.colored_label(ui.visuals().error_fg_color, n);
            if ui.button("Retry").clicked() {
                out.push(Intent::Retry(tab.id));
            }
        } else {
            ui.label("Loading…");
        }
        return out;
    };
    egui::ScrollArea::both().show(ui, |ui| {
        ui.strong("Columns");
        grid(ui, "structure_columns", 4, |ui| {
            row(ui, true, ["Column", "Type", "Attributes", "Comment"]);
            for c in &s.columns {
                let m = &c.meta;
                let mut attrs = vec![];
                if m.is_primary_key {
                    attrs.push("PK".to_string());
                }
                if m.nullable == Some(false) {
                    attrs.push("NOT NULL".into());
                }
                if let Some(d) = &c.default {
                    attrs.push(format!("DEFAULT {d}"));
                }
                ui.strong(&m.name);
                row(
                    ui,
                    false,
                    [
                        &m.type_name,
                        &or_dash(attrs.join(", ")),
                        c.comment.as_deref().unwrap_or(""),
                    ],
                );
            }
        });
        ui.add_space(12.0);
        ui.strong("Indexes");
        grid(ui, "structure_indexes", 3, |ui| {
            row(ui, true, ["Name", "Columns", "Attributes"]);
            for i in &s.indexes {
                let mut attrs = vec![];
                if i.primary {
                    attrs.push("PRIMARY".to_string());
                } else if i.unique {
                    attrs.push("UNIQUE".into());
                }
                if let Some(m) = &i.method {
                    attrs.push(format!("USING {m}"));
                }
                row(
                    ui,
                    false,
                    [&i.name, &i.columns.join(", "), &or_dash(attrs.join(", "))],
                );
            }
        });
        ui.add_space(12.0);
        ui.strong("Foreign keys");
        grid(ui, "structure_fks", 5, |ui| {
            row(
                ui,
                true,
                ["Name", "Columns", "References", "On update", "On delete"],
            );
            for f in &s.fks {
                row(
                    ui,
                    false,
                    [
                        &f.name,
                        &f.columns.join(", "),
                        &format!("{} ({})", f.ref_table.name, f.ref_columns.join(", ")),
                        &f.on_update,
                        &f.on_delete,
                    ],
                );
            }
        });
    });
    out
}

fn or_dash(s: String) -> String {
    if s.is_empty() { "-".into() } else { s }
}

fn grid(ui: &mut egui::Ui, id: &str, cols: usize, f: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id)
        .striped(true)
        .num_columns(cols)
        .show(ui, f);
}

/// Ends a grid row; the first cell may already have been added by the caller.
fn row<const N: usize>(ui: &mut egui::Ui, header: bool, cells: [&str; N]) {
    for c in cells {
        if header {
            ui.strong(c);
        } else {
            ui.label(c);
        }
    }
    ui.end_row();
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crustdb_core::model::{
        ColumnDetail, ColumnMeta, ForeignKeyInfo, IndexInfo, ObjectKind, ObjectRef, TypeCategory,
    };
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;
    use crate::runtime::TabId;
    use crate::state::Structure;

    fn obj(name: &str) -> ObjectRef {
        ObjectRef {
            database: "d".into(),
            schema: None,
            name: name.into(),
            kind: ObjectKind::Table,
        }
    }

    fn column(name: &str, ty: &str, pk: bool, nullable: bool) -> ColumnDetail {
        ColumnDetail {
            meta: ColumnMeta {
                name: name.into(),
                type_name: ty.into(),
                category: TypeCategory::Other,
                nullable: Some(nullable),
                is_primary_key: pk,
            },
            default: None,
            comment: None,
        }
    }

    #[test]
    fn structure_renders_columns_indexes_fks_as_tables() {
        // Arrange
        let mut tab = TableTab::new(TabId(1), obj("t"), 300);
        tab.apply_structure(Structure {
            columns: vec![
                ColumnDetail {
                    default: Some("0".into()),
                    comment: Some("row id".into()),
                    ..column("id", "int", true, false)
                },
                column("_deleted_at", "datetime(6)", false, true),
            ],
            indexes: vec![IndexInfo {
                name: "pk_t".into(),
                columns: vec!["id".into()],
                unique: true,
                primary: true,
                method: None,
            }],
            fks: vec![ForeignKeyInfo {
                name: "fk_x".into(),
                columns: vec!["a".into()],
                ref_table: obj("other"),
                ref_columns: vec!["id".into()],
                on_update: "CASCADE".into(),
                on_delete: "SET NULL".into(),
            }],
        });
        let mut h = Harness::new_ui(|ui| {
            show(ui, &tab);
        });

        // Act
        h.run();

        // Assert
        for l in [
            "Column",
            "Type",
            "Attributes",
            "id",
            "int",
            "PK, NOT NULL, DEFAULT 0",
            "Comment",
            "row id",
            "pk_t",
            "PRIMARY",
            "On update",
            "On delete",
            "_deleted_at",
            "datetime(6)",
            "-",
            "Name",
            "Columns",
            "fk_x",
            "a",
            "other (id)",
            "CASCADE",
            "SET NULL",
        ] {
            assert!(h.query_all_by_label(l).next().is_some(), "missing {l}");
        }
    }
}
