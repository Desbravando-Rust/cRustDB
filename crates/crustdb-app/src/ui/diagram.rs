//! Read-only ER diagram window: one box per table, one line per foreign key.
use eframe::egui;

use crate::runtime::DiagramTable;
use crate::state::diagram::{Diagram, edges, layout};

/// Draws the diagram window; closing it clears `diagram`.
pub fn show(ctx: &egui::Context, diagram: &mut Option<Diagram>) {
    let Some(d) = diagram.as_ref() else {
        return;
    };
    let mut open = true;
    egui::Window::new("ER diagram")
        .open(&mut open)
        .default_size([800.0, 600.0])
        .show(ctx, |ui| {
            ui.strong(d.title.as_str());
            match &d.tables {
                _ if d.failed => {
                    ui.label("Could not load the diagram.");
                }
                None => {
                    ui.label("Loading…");
                }
                Some(t) if t.is_empty() => {
                    ui.label("No tables.");
                }
                Some(t) => {
                    egui::ScrollArea::both().show(ui, |ui| draw(ui, t));
                }
            }
        });
    if !open {
        *diagram = None;
    }
}

fn draw(ui: &mut egui::Ui, tables: &[DiagramTable]) {
    let rects = layout(tables);
    let size = rects
        .iter()
        .fold(egui::Vec2::ZERO, |s, r| s.max(r.max.to_vec2()));
    let (resp, painter) = ui.allocate_painter(size, egui::Sense::hover());
    let offset = resp.rect.min.to_vec2();
    let fill = ui.visuals().window_fill;
    let stroke = ui.visuals().widgets.noninteractive.fg_stroke;
    // Lines go first so the boxes cover their ends.
    for (a, b) in edges(tables) {
        let (a, b) = (rects[a].translate(offset), rects[b].translate(offset));
        painter.line_segment([a.center(), b.center()], stroke);
    }
    for (t, r) in tables.iter().zip(&rects) {
        let r = r.translate(offset);
        painter.rect_filled(r, 4.0, fill);
        painter.rect_stroke(r, 4.0, stroke, egui::StrokeKind::Inside);
        ui.scope_builder(egui::UiBuilder::new().max_rect(r.shrink(4.0)), |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add(egui::Label::new(egui::RichText::new(&t.object.name).strong()).truncate());
            for c in &t.columns {
                let pk = if c.is_primary_key { " PK" } else { "" };
                let text = format!("{} {}{pk}", c.name, c.type_name);
                ui.add(egui::Label::new(text).truncate());
            }
        });
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crustdb_core::model::{ColumnMeta, ForeignKeyInfo, ObjectKind, ObjectRef, TypeCategory};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;
    use crate::runtime::{DiagramTable, RequestId};

    fn obj(name: &str) -> ObjectRef {
        ObjectRef {
            database: "d".into(),
            schema: Some("public".into()),
            name: name.into(),
            kind: ObjectKind::Table,
        }
    }

    fn col(name: &str, pk: bool) -> ColumnMeta {
        ColumnMeta {
            name: name.into(),
            type_name: "int4".into(),
            category: TypeCategory::Integer,
            nullable: Some(false),
            is_primary_key: pk,
        }
    }

    #[test]
    fn diagram_window_draws_tables_and_columns() {
        // Arrange
        let fk = ForeignKeyInfo {
            name: "fk".into(),
            columns: vec!["customer_id".into()],
            ref_table: obj("customers"),
            ref_columns: vec!["id".into()],
            on_update: "NO ACTION".into(),
            on_delete: "NO ACTION".into(),
        };
        let mut d = Some(Diagram::new(RequestId(1), "d.public".into()));
        d.as_mut().unwrap().apply(Some(vec![
            DiagramTable {
                object: obj("customers"),
                columns: vec![col("id", true)],
                fks: vec![],
            },
            DiagramTable {
                object: obj("orders"),
                columns: vec![col("order_id", true), col("customer_id", false)],
                fks: vec![fk],
            },
        ]));
        let mut h = Harness::new_ui(|ui| show(ui.ctx(), &mut d));

        // Act
        h.run();

        // Assert
        h.get_by_label("customers");
        h.get_by_label("orders");
        h.get_by_label("id int4 PK");
        h.get_by_label("customer_id int4");
    }
}
