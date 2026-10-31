//! Command line, Run/Cancel buttons and reply grid of a key-value workspace.
use eframe::egui;

use super::Intent;
use super::grid::grid;
use crate::state::command_pane::CommandPane;

/// Draws and returns intents; running, cancelling and the reply all go through the runtime.
pub fn show(ui: &mut egui::Ui, c: &mut CommandPane) -> Vec<Intent> {
    let mut out = vec![];
    ui.horizontal(|ui| {
        let r = ui.add(
            egui::TextEdit::singleline(&mut c.text)
                .hint_text("command, e.g. GET user:1")
                .desired_width((ui.available_width() - 120.0).max(80.0)),
        );
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let running = c.is_running();
        if (ui.add_enabled(!running, egui::Button::new("Run")).clicked() || enter) && !running {
            out.push(Intent::RunCommand);
        }
        if running && ui.button("Cancel").clicked() {
            out.push(Intent::CancelCommand);
        }
    });
    if let Some(e) = &c.error {
        ui.colored_label(ui.visuals().error_fg_color, e);
    }
    if !c.columns.is_empty() {
        grid(
            ui,
            &c.columns,
            &c.rows,
            None,
            &mut c.selected,
            &mut c.anchor,
            &[],
            None,
            &[],
            None,
        );
    }
    out
}
