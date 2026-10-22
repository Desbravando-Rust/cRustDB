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
        for c in &s.columns {
            let m = &c.meta;
            ui.monospace(format!(
                "{}{} {}{}{}{}",
                m.name,
                if m.is_primary_key { " [PK]" } else { "" },
                m.type_name,
                if m.nullable == Some(false) {
                    " NOT NULL"
                } else {
                    ""
                },
                c.default
                    .as_ref()
                    .map_or(String::new(), |d| format!(" DEFAULT {d}")),
                c.comment
                    .as_ref()
                    .map_or(String::new(), |d| format!(" -- {d}")),
            ));
        }
        ui.strong("Indexes");
        for i in &s.indexes {
            ui.monospace(format!(
                "{} ({}){}{}",
                i.name,
                i.columns.join(", "),
                if i.primary {
                    " PRIMARY"
                } else if i.unique {
                    " UNIQUE"
                } else {
                    ""
                },
                i.method
                    .as_ref()
                    .map_or(String::new(), |m| format!(" USING {m}")),
            ));
        }
        ui.strong("Foreign keys");
        for f in &s.fks {
            ui.monospace(format!(
                "{} ({}) -> {} ({}) ON UPDATE {} ON DELETE {}",
                f.name,
                f.columns.join(", "),
                f.ref_table.name,
                f.ref_columns.join(", "),
                f.on_update,
                f.on_delete,
            ));
        }
    });
    out
}
