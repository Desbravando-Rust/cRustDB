//! Top strip: product logo and the connection badge of the active workspace.
use eframe::egui;

use crate::state::AppState;

pub fn show(ui: &mut egui::Ui, state: &AppState) {
    ui.horizontal(|ui| {
        ui.add(
            egui::Image::new(egui::include_image!("../../../../assets/imgs/logo2.png"))
                .fit_to_exact_size(egui::vec2(20.0, 20.0))
                .alt_text("cRustDB top bar logo"),
        );
        if let Some(w) = state.active_online() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                egui::Frame::new()
                    .fill(egui::Color32::from_rgb(20, 60, 40))
                    .corner_radius(10.0)
                    .inner_margin(egui::Margin::symmetric(8, 2))
                    .show(ui, |ui| {
                        ui.colored_label(
                            egui::Color32::from_rgb(34, 197, 94),
                            format!(
                                "{} Connected: {}",
                                egui_phosphor::regular::CIRCLE,
                                w.config.name
                            ),
                        );
                    });
            });
        }
    });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;
    use crate::state::Status;
    use crate::state::workspace::tests::state;
    use crate::ui::home::tests::ready;
    use crustdb_core::model::{Product, ServerVersion};

    fn run(s: &AppState) -> Option<()> {
        let mut done = false;
        let mut h = Harness::new_ui(|ui| {
            if ready(ui, &mut done) {
                show(ui, s);
            }
        });
        h.run();
        h.query_by_label_contains("Connected: alpha").map(|_| ())
    }

    #[test]
    fn top_bar_shows_connected_badge() {
        // Arrange
        let mut s = state(&["alpha"]);
        let ws = s.open_workspace(s.config.connections[0].clone());
        s.workspaces[0].status = Status::Online(ServerVersion {
            product: Product::Postgres,
            major: 16,
            minor: 0,
            patch: 0,
            raw: "16.0".into(),
        });
        s.active = Some(ws);

        // Act & Assert
        assert!(run(&s).is_some());
    }

    #[test]
    fn top_bar_hides_badge_when_offline() {
        // Arrange
        let s = state(&["alpha"]);

        // Act & Assert
        assert!(run(&s).is_none());
    }
}
