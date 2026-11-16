//! Accessible names for inputs that have no visible label next to them.
use eframe::egui::{self, Role};

/// Names a text input for screen readers; the name must be unique among visible widgets.
pub fn name_input(r: &egui::Response, role: Role, name: impl Into<String>) {
    let name = name.into();
    r.widget_info(|| egui::WidgetInfo::labeled(role, r.enabled(), name.clone()));
}

/// Names a plain single-line text input.
pub fn name_text(r: &egui::Response, name: impl Into<String>) {
    name_input(r, Role::TextInput, name);
}
