//! Read-only ER diagram window: one box per table, one line per foreign key.
use eframe::egui;

use crate::state::diagram::Diagram;

/// Draws the diagram window; closing it clears `diagram`.
pub fn show(ctx: &egui::Context, diagram: &mut Option<Diagram>) {
    let _ = (ctx, diagram);
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
        h.run();
        h.get_by_label("customers");
        h.get_by_label("orders");
        h.get_by_label("id int4 PK");
        h.get_by_label("customer_id int4");
    }
}
