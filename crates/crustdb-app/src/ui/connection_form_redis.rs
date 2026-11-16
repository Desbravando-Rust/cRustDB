//! Redis-specific rows of the connection form.
use eframe::egui;

use super::connection_form::error_for;
use crate::state::connection_form::ConnectionForm;

/// Optional text field: empty means "unset" (`None`).
fn optional_edit(ui: &mut egui::Ui, v: &mut Option<String>, hint: &str, label: egui::Id) {
    let mut s = v.clone().unwrap_or_default();
    let edit = egui::TextEdit::singleline(&mut s).hint_text(hint);
    let r = ui.add(edit).labelled_by(label);
    if r.changed() {
        *v = (!s.is_empty()).then_some(s);
    }
}

pub fn fields(ui: &mut egui::Ui, f: &mut ConnectionForm) {
    let l = ui.label("Database index");
    optional_edit(ui, &mut f.draft.database, "0", l.id);
    ui.end_row();
    error_for(ui, f, "database");
    ui.end_row();

    let l = ui
        .label("Key separator")
        .on_hover_text("Splits keys into folders in the key tree. Empty uses \":\".");
    optional_edit(ui, &mut f.draft.key_separator, ":", l.id);
    ui.end_row();
    error_for(ui, f, "key_separator");
    ui.end_row();
}
