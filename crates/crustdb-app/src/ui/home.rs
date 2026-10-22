use eframe::egui;
use egui_phosphor::regular as icon;

use super::Intent;
use crate::state::{AppState, Status};

const ACTIVE: egui::Color32 = egui::Color32::from_rgb(90, 190, 110);

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
        let live = state
            .live_workspace(c.id)
            .and_then(|id| state.workspaces.iter().find(|w| w.id == id));
        let connecting = live.is_some_and(|w| w.status == Status::Connecting);
        ui.horizontal(|ui| {
            let name = egui::RichText::new(&c.name).strong();
            if ui.selectable_label(false, name).double_clicked() && !connecting {
                out.push(Intent::Connect(c.id));
            }
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
                if let Some(w) = live.filter(|w| matches!(w.status, Status::Online(_))) {
                    if ui.button("Open").clicked() {
                        out.push(Intent::SwitchWorkspace(w.id));
                    }
                    ui.colored_label(ACTIVE, "Active");
                    return;
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

    use crustdb_core::model::{Product, ServerVersion};

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
        let mut h = Harness::new_ui(|ui| {
            egui_extras::install_image_loaders(ui.ctx());
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

    #[test]
    fn double_click_connects() {
        let s = state(&["alpha"]);
        let id = s.config.connections[0].id;
        let mut intents = vec![];
        let mut h = Harness::builder()
            .with_step_dt(0.05)
            .build_ui(|ui| intents.extend(show(ui, &s)));
        h.run();
        h.get_by_label("alpha").click();
        h.step();
        h.get_by_label("alpha").click();
        h.step();
        h.run();
        drop(h);
        assert!(intents.contains(&Intent::Connect(id)));
    }

    #[test]
    fn live_connection_shows_open_instead_of_connect() {
        let mut s = state(&["alpha"]);
        let ws = s.open_workspace(s.config.connections[0].clone());
        s.workspaces[0].status = Status::Online(ServerVersion {
            product: Product::Postgres,
            major: 16,
            minor: 0,
            patch: 0,
            raw: "16.0".into(),
        });
        let mut intents = vec![];
        let mut h = Harness::new_ui(|ui| intents.extend(show(ui, &s)));
        h.run();
        h.get_by_label("Active");
        assert!(h.query_by_label("Connect").is_none());
        h.get_by_label("Open").click();
        h.run();
        drop(h);
        assert!(intents.contains(&Intent::SwitchWorkspace(ws)));
    }
}
