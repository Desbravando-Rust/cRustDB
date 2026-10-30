//! Redis-specific rows of the connection form.
use eframe::egui;

use super::connection_form::error_for;
use crate::state::connection_form::ConnectionForm;

/// Optional text field: empty means "unset" (`None`).
fn optional_edit(ui: &mut egui::Ui, v: &mut Option<String>, hint: &str) {
    let mut s = v.clone().unwrap_or_default();
    let edit = egui::TextEdit::singleline(&mut s).hint_text(hint);
    if ui.add(edit).changed() {
        *v = (!s.is_empty()).then_some(s);
    }
}

pub fn fields(ui: &mut egui::Ui, f: &mut ConnectionForm) {
    ui.label("Database index");
    optional_edit(ui, &mut f.draft.database, "0");
    ui.end_row();
    error_for(ui, f, "database");
    ui.end_row();

    ui.label("Key separator")
        .on_hover_text("Splits keys into folders in the key tree. Empty uses \":\".");
    optional_edit(ui, &mut f.draft.key_separator, ":");
    ui.end_row();
    error_for(ui, f, "key_separator");
    ui.end_row();
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crustdb_core::model::DriverKind;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;

    #[test]
    fn redis_form_shows_index_and_separator_instead_of_database() {
        // Arrange
        let mut f = ConnectionForm::new(DriverKind::Redis, true);
        let mut h = Harness::builder()
            .with_accessibility_check(false)
            .build_ui(|ui| {
                super::super::connection_form::show(ui, &mut f, &[]);
            });

        // Act
        h.run();

        // Assert
        h.get_by_label("Database index");
        h.get_by_label("Key separator");
        assert!(h.query_by_label("Database").is_none());
    }
}
