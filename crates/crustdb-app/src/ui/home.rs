use eframe::egui;
use egui_phosphor::regular as icon;

use super::Intent;
use crate::state::{AppState, Status};

pub fn show(ui: &mut egui::Ui, state: &AppState) -> Vec<Intent> {
    let mut out = vec![];
    ui.add(
        egui::Image::new(egui::include_image!("../../../../assets/imgs/logo2.png"))
            .max_height(96.0)
            .alt_text("cRustDB logo"),
    );
    ui.horizontal(|ui| {
        ui.heading("cRustDB");
        if ui.button(format!("{} New", icon::PLUS)).clicked() {
            out.push(Intent::NewConnection);
        }
        if ui.button(format!("{} Import URL", icon::LINK)).clicked() {
            out.push(Intent::ImportUrl);
        }
    });
    if let Some(w) = &state.config.warning {
        ui.colored_label(ui.visuals().warn_fg_color, w);
    }
    ui.separator();
    if state.config.connections.is_empty() {
        ui.weak("No saved connections. Create one with New or Import URL.");
    }
    for c in &state.config.connections {
        let ws = state.workspaces.iter().rev().find(|w| w.config.id == c.id);
        let connecting = ws.is_some_and(|w| w.status == Status::Connecting);
        ui.horizontal(|ui| {
            ui.strong(&c.name);
            ui.weak(format!("{}@{}:{}", c.user, c.host, c.port));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(!state.config.read_only, egui::Button::new(icon::TRASH))
                    .on_hover_text("Delete")
                    .clicked()
                {
                    out.push(Intent::DeleteConnection(c.id));
                }
                if ui
                    .button(icon::PENCIL_SIMPLE)
                    .on_hover_text("Edit")
                    .clicked()
                {
                    out.push(Intent::EditConnection(c.id));
                }
                let label = if connecting {
                    "Connecting..."
                } else {
                    "Connect"
                };
                if ui
                    .add_enabled(!connecting, egui::Button::new(label))
                    .clicked()
                {
                    out.push(Intent::Connect(c.id));
                }
            });
        });
        if let Some(Status::Failed(e)) = ws.map(|w| &w.status) {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;
    use crate::state::workspace::tests::state;

    #[test]
    fn home_lists_saved_connections() {
        let s = state(&["alpha", "beta"]);
        let mut fonts_ready = false;
        let mut h = Harness::new_ui(|ui| {
            // Fonts apply on the pass after `set_fonts`; skip drawing until then.
            if !std::mem::replace(&mut fonts_ready, true) {
                crate::ui::theme::install_theme(ui.ctx());
                // Without loaders the logo renders egui's "⚠️" error glyph,
                // which the themed fonts cannot draw.
                egui_extras::install_image_loaders(ui.ctx());
                return;
            }
            show(ui, &s);
        });
        h.run();
        h.get_by_label("alpha");
        h.get_by_label("beta");
    }

    #[test]
    fn home_shows_logo() {
        let s = state(&[]);
        let mut fonts_ready = false;
        let mut h = Harness::new_ui(|ui| {
            // Icons need the themed fonts, which apply on the next pass.
            if !std::mem::replace(&mut fonts_ready, true) {
                crate::ui::theme::install_theme(ui.ctx());
                egui_extras::install_image_loaders(ui.ctx());
                return;
            }
            show(ui, &s);
        });
        // `run` waits for pending images, so the decode must be done by now.
        h.run();
        h.get_by_label("cRustDB logo");
        let poll = h.ctx.try_load_image(
            "bytes://../../../../assets/imgs/logo2.png",
            egui::SizeHint::default(),
        );
        assert!(matches!(poll, Ok(egui::load::ImagePoll::Ready { .. })));
    }
}
