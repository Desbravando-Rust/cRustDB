use eframe::egui;

use super::Intent;
use crate::state::TableTab;
use crate::state::filter_draft::label;

fn named(r: &egui::Response, name: &'static str, selected: &str) {
    r.widget_info(|| {
        let mut i = egui::WidgetInfo::labeled(egui::Role::ComboBox, r.enabled(), name);
        i.current_text_value = Some(selected.into());
        i
    });
}

pub fn show(ui: &mut egui::Ui, tab: &mut TableTab) -> Vec<Intent> {
    let mut out = vec![];
    let mut remove = None;
    let names: Vec<String> = tab.columns.iter().map(|c| c.name.clone()).collect();
    for i in 0..tab.drafts.len() {
        let ops = tab.drafts[i].operators(&tab.columns, tab.driver);
        let d = &mut tab.drafts[i];
        ui.horizontal(|ui| {
            let mut picked = None;
            let col = egui::ComboBox::from_id_salt(("fcol", tab.id.0, i))
                .selected_text(d.column.clone())
                .show_ui(ui, |ui| {
                    for n in &names {
                        if ui.selectable_label(d.column == *n, n).clicked() {
                            picked = Some(n.clone());
                        }
                    }
                });
            named(&col.response, "Filter column", &d.column);
            if let Some(n) = picked {
                d.set_column(&tab.columns, tab.driver, &n);
            }
            let op = egui::ComboBox::from_id_salt(("fop", tab.id.0, i))
                .selected_text(label(d.op))
                .show_ui(ui, |ui| {
                    for op in ops {
                        ui.selectable_value(&mut d.op, *op, label(*op));
                    }
                });
            named(&op.response, "Filter operator", label(d.op));
            let r = ui.text_edit_singleline(&mut d.input);
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                out.push(Intent::ApplyFilters(tab.id));
            }
            if ui.button("−").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        tab.remove_draft(i);
    }
    ui.horizontal(|ui| {
        if ui.button("+ Filter").clicked() {
            tab.add_draft();
        }
        if ui.button("Apply").clicked() {
            out.push(Intent::ApplyFilters(tab.id));
        }
        if ui.button("Clear").clicked() {
            out.push(Intent::ClearFilters(tab.id));
        }
        if let Some(e) = tab.bar_error() {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
    });
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crustdb_core::model::{ObjectKind, ObjectRef};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;
    use crate::runtime::TabId;

    #[test]
    fn filter_combos_have_accessible_names() {
        // Arrange
        let object = ObjectRef {
            database: "d".into(),
            schema: Some("public".into()),
            name: "t".into(),
            kind: ObjectKind::Table,
        };
        let mut tab = TableTab::new(TabId(1), object, 100);
        tab.add_draft();
        let mut h = Harness::builder()
            .with_accessibility_check(false) // value text input is unnamed (#186)
            .build_ui(|ui| {
                show(ui, &mut tab);
            });

        // Act
        h.run();

        // Assert
        for name in ["Filter column", "Filter operator"] {
            h.get_by_role_and_label(egui::accesskit::Role::ComboBox, name);
        }
    }
}
