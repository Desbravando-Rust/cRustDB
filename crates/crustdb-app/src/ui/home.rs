use crustdb_core::model::DriverKind;
use eframe::egui;
use egui_phosphor::regular as icon;

use super::Intent;
use crate::state::{AppState, Status};

const ACTIVE: egui::Color32 = egui::Color32::from_rgb(90, 190, 110);
const POSTGRES_BLUE: egui::Color32 = egui::Color32::from_rgb(80, 140, 210);
const MYSQL_ORANGE: egui::Color32 = egui::Color32::from_rgb(230, 140, 40);

fn engine_color(kind: DriverKind) -> egui::Color32 {
    match kind {
        DriverKind::Postgres => POSTGRES_BLUE,
        DriverKind::Mysql => MYSQL_ORANGE,
    }
}

fn engine_name(kind: DriverKind) -> &'static str {
    match kind {
        DriverKind::Postgres => "PostgreSQL",
        DriverKind::Mysql => "MySQL",
    }
}

const BOOK_URL: &str = "https://desbravandorust.com.br/";

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
            let engine = egui::RichText::new(icon::DATABASE).color(engine_color(c.driver));
            ui.label(engine).on_hover_text(engine_name(c.driver));
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
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        ui.horizontal(|ui| {
            ui.add(
                egui::Image::new(egui::include_image!(
                    "../../../../assets/imgs/book-cover.png"
                ))
                .max_height(48.0)
                .alt_text("Desbravando Rust book cover"),
            );
            ui.vertical(|ui| {
                ui.weak(
                    "This app is built in Rust. Coming from Python? Learn it with Desbravando Rust.",
                );
                ui.hyperlink_to(format!("Read a free chapter {}", icon::ARROW_RIGHT), BOOK_URL);
            });
        });
        ui.separator();
    });
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

    /// Installs theme fonts and image loaders on the first pass and returns
    /// `false` then: fonts apply on the next pass, and without loaders the
    /// logo renders egui's "⚠️" error glyph, which no installed font draws.
    fn ready(ui: &egui::Ui, done: &mut bool) -> bool {
        if std::mem::replace(done, true) {
            return true;
        }
        crate::ui::theme::install_theme(ui.ctx());
        egui_extras::install_image_loaders(ui.ctx());
        false
    }

    #[test]
    fn home_lists_saved_connections() {
        let s = state(&["alpha", "beta"]);
        let mut done = false;
        let mut h = Harness::new_ui(|ui| {
            if ready(ui, &mut done) {
                show(ui, &s);
            }
        });
        h.run();
        h.get_by_label("alpha");
        h.get_by_label("beta");
    }

    #[test]
    fn home_shows_logo() {
        let s = state(&[]);
        let mut done = false;
        let mut h = Harness::new_ui(|ui| {
            if ready(ui, &mut done) {
                show(ui, &s);
            }
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
    fn home_shows_book_promo() {
        let s = state(&[]);
        let mut done = false;
        let mut h = Harness::new_ui(|ui| {
            if ready(ui, &mut done) {
                show(ui, &s);
            }
        });
        h.run();
        h.get_by_label(
            "This app is built in Rust. Coming from Python? Learn it with Desbravando Rust.",
        );
        h.get_by_label(&format!("Read a free chapter {}", icon::ARROW_RIGHT));
        h.get_by_label("Desbravando Rust book cover");
        let poll = h.ctx.try_load_image(
            "bytes://../../../../assets/imgs/book-cover.png",
            egui::SizeHint::default(),
        );
        assert!(matches!(poll, Ok(egui::load::ImagePoll::Ready { .. })));
    }

    #[test]
    fn book_promo_links_to_site() {
        let s = state(&[]);
        let mut done = false;
        let mut h = Harness::new_ui(|ui| {
            if ready(ui, &mut done) {
                show(ui, &s);
            }
        });
        h.run();
        h.get_by_label(&format!("Read a free chapter {}", icon::ARROW_RIGHT))
            .click();
        h.step();
        let opened = h.output().platform_output.commands.iter().any(|c| {
            matches!(c, egui::OutputCommand::OpenUrl(u) if u.url == "https://desbravandorust.com.br/")
        });
        assert!(opened);
    }

    #[test]
    fn double_click_connects() {
        let s = state(&["alpha"]);
        let id = s.config.connections[0].id;
        let mut intents = vec![];
        let mut done = false;
        let mut h = Harness::builder().with_step_dt(0.05).build_ui(|ui| {
            if ready(ui, &mut done) {
                intents.extend(show(ui, &s));
            }
        });
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
        let mut done = false;
        let mut h = Harness::new_ui(|ui| {
            if ready(ui, &mut done) {
                intents.extend(show(ui, &s));
            }
        });
        h.run();
        h.get_by_label("Active");
        assert!(h.query_by_label("Connect").is_none());
        h.get_by_label("Open").click();
        h.run();
        drop(h);
        assert!(intents.contains(&Intent::SwitchWorkspace(ws)));
    }

    #[test]
    fn engine_colors_differ() {
        assert_ne!(
            engine_color(DriverKind::Postgres),
            engine_color(DriverKind::Mysql)
        );
    }
}
