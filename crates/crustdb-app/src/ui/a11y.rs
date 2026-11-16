//! Accessible names for inputs that have no visible label next to them.
use eframe::egui::{self, Role};

/// Names a text input for screen readers; the name must be unique among visible widgets.
/// Patches the widget's existing accesskit node, keeping the value `TextEdit` reports.
pub fn name_input(r: &egui::Response, role: Role, name: impl Into<String>) -> &egui::Response {
    let name = name.into();
    r.ctx.accesskit_node_builder(r.id, |b| {
        b.set_role(role);
        b.set_label(name);
    });
    r
}

/// Names a plain single-line text input.
pub fn name_text(r: &egui::Response, name: impl Into<String>) {
    name_input(r, Role::TextInput, name);
}

/// Names the SQL editor's multiline input.
pub fn name_sql_editor(r: &egui::Response) -> &egui::Response {
    name_input(r, Role::MultilineTextInput, "SQL editor")
}
