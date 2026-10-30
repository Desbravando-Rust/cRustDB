use crustdb_core::model::DriverKind;
use eframe::egui;
use egui_phosphor::regular as icon;

use super::{Intent, labels};
use crate::state::{AppState, Status};

const ACTIVE: egui::Color32 = egui::Color32::from_rgb(90, 190, 110);
const POSTGRES_BLUE: egui::Color32 = egui::Color32::from_rgb(80, 140, 210);
const MYSQL_ORANGE: egui::Color32 = egui::Color32::from_rgb(230, 140, 40);
const SQLITE_TEAL: egui::Color32 = egui::Color32::from_rgb(60, 170, 180);

fn engine_color(kind: DriverKind) -> egui::Color32 {
    match kind {
        DriverKind::Postgres => POSTGRES_BLUE,
        DriverKind::Mysql => MYSQL_ORANGE,
        DriverKind::Sqlite => SQLITE_TEAL,
    }
}

fn engine_name(kind: DriverKind) -> &'static str {
    match kind {
        DriverKind::Postgres => "PostgreSQL",
        DriverKind::Mysql => "MySQL",
        DriverKind::Sqlite => "SQLite",
    }
}

const BOOK_URL: &str = "https://desbravandorust.com.br/";

pub(crate) fn logo(ui: &mut egui::Ui, max_height: f32) {
    ui.add(
        egui::Image::new(egui::include_image!("../../../../assets/imgs/logo2.png"))
            .max_height(max_height)
            .alt_text("cRustDB logo"),
    );
}

pub fn show(ui: &mut egui::Ui, state: &AppState) -> Vec<Intent> {
    let mut out = vec![];
    egui::Panel::bottom("home_book_promo")
        .resizable(false)
        .show(ui, |ui| {
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
                        "Este app é feito em Rust. Vindo do Python? Aprenda com o Desbravando Rust.",
                    );
                    ui.hyperlink_to(format!("Leia um capítulo grátis {}", icon::ARROW_RIGHT), BOOK_URL);
                });
            });
        });
    egui::ScrollArea::vertical().show(ui, |ui| {
        if state.active_online().is_none() {
            logo(ui, 96.0);
        }
        let mut filter = None;
        ui.horizontal(|ui| {
            ui.heading("cRustDB");
            if ui.button(format!("{} New", icon::PLUS)).clicked() {
                out.push(Intent::NewConnection);
            }
            out.extend(labels::editor(
                ui,
                &state.config.labels,
                state.config.read_only,
            ));
            filter = labels::filter_combo(ui, &state.config.labels);
        });
        if let Some(w) = &state.config.warning {
            ui.colored_label(ui.visuals().warn_fg_color, w);
        }
        ui.separator();
        if state.config.connections.is_empty() {
            ui.weak("No saved connections. Create one with New.");
        }
        let shown =
            (state.config.connections.iter()).filter(|c| labels::matches(c, filter.as_deref()));
        for c in shown {
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
                for n in &c.labels {
                    labels::chip(ui, n, &state.config.labels);
                }
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
    });
    out
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use crustdb_core::model::{Product, ServerVersion};

    use super::*;
    use crate::state::workspace::tests::state;

    /// Installs theme fonts and image loaders on the first pass and returns
    /// `false` then: fonts apply on the next pass, and without loaders the
    /// logo renders egui's "⚠️" error glyph, which no installed font draws.
    pub(crate) fn ready(ui: &egui::Ui, done: &mut bool) -> bool {
        if std::mem::replace(done, true) {
            return true;
        }
        crate::ui::theme::install_theme(ui.ctx());
        egui_extras::install_image_loaders(ui.ctx());
        false
    }

    #[test]
    fn home_lists_saved_connections() {
        // Arrange
        let s = state(&["alpha", "beta"]);
        let mut done = false;
        let mut h = Harness::new_ui(|ui| {
            if ready(ui, &mut done) {
                show(ui, &s);
            }
        });

        // Act
        h.run();

        // Assert
        h.get_by_label("alpha");
        h.get_by_label("beta");
    }

    #[test]
    fn home_has_single_new_button() {
        // Arrange
        let s = state(&[]);
        let mut done = false;
        let mut h = Harness::new_ui(|ui| {
            if ready(ui, &mut done) {
                show(ui, &s);
            }
        });

        // Act
        h.run();

        // Assert
        h.get_by_label(&format!("{} New", icon::PLUS));
        assert!(h.query_by_label_contains("Import URL").is_none());
    }

    #[test]
    fn home_shows_logo() {
        // Arrange
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

        // Act
        let poll = h.ctx.try_load_image(
            "bytes://../../../../assets/imgs/logo2.png",
            egui::SizeHint::default(),
        );

        // Assert
        assert!(matches!(poll, Ok(egui::load::ImagePoll::Ready { .. })));
    }

    #[test]
    fn home_shows_book_promo() {
        // Arrange
        let s = state(&[]);
        let mut done = false;
        let mut h = Harness::new_ui(|ui| {
            if ready(ui, &mut done) {
                show(ui, &s);
            }
        });
        h.run();
        h.get_by_label(
            "Este app é feito em Rust. Vindo do Python? Aprenda com o Desbravando Rust.",
        );
        h.get_by_label(&format!("Leia um capítulo grátis {}", icon::ARROW_RIGHT));
        h.get_by_label("Desbravando Rust book cover");

        // Act
        let poll = h.ctx.try_load_image(
            "bytes://../../../../assets/imgs/book-cover.png",
            egui::SizeHint::default(),
        );

        // Assert
        assert!(matches!(poll, Ok(egui::load::ImagePoll::Ready { .. })));
    }

    #[test]
    fn book_promo_links_to_site() {
        // Arrange
        let s = state(&[]);
        let mut done = false;
        let mut h = Harness::new_ui(|ui| {
            if ready(ui, &mut done) {
                show(ui, &s);
            }
        });
        h.run();
        h.get_by_label(&format!("Leia um capítulo grátis {}", icon::ARROW_RIGHT))
            .click();
        h.step();

        // Act
        let opened = h.output().platform_output.commands.iter().any(|c| {
            matches!(c, egui::OutputCommand::OpenUrl(u) if u.url == "https://desbravandorust.com.br/")
        });

        // Assert
        assert!(opened);
    }

    #[test]
    fn book_promo_stays_visible_when_list_overflows() {
        // Arrange
        let names: Vec<String> = (0..15).map(|i| format!("conn{i}")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let s = state(&names);
        let mut done = false;
        let mut h = Harness::builder()
            .with_size(egui::vec2(800.0, 300.0))
            .build_ui(|ui| {
                if ready(ui, &mut done) {
                    show(ui, &s);
                }
            });
        h.run();

        // Act
        let r = h
            .get_by_label(&format!("Leia um capítulo grátis {}", icon::ARROW_RIGHT))
            .rect();

        // Assert
        assert!(r.max.y <= 300.0, "promo cut off: {r:?}");
    }

    #[test]
    fn double_click_connects() {
        // Arrange
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

        // Act
        h.run();
        drop(h);

        // Assert
        assert!(intents.contains(&Intent::Connect(id)));
    }

    #[test]
    fn live_connection_shows_open_instead_of_connect() {
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
        let mut intents = vec![];
        let mut done = false;
        let mut h = Harness::new_ui(|ui| {
            if ready(ui, &mut done) {
                intents.extend(show(ui, &s));
            }
        });
        h.run();

        // Act & Assert
        h.get_by_label("Active");
        assert!(h.query_by_label("Connect").is_none());
        h.get_by_label("Open").click();
        h.run();
        drop(h);
        assert!(intents.contains(&Intent::SwitchWorkspace(ws)));
    }

    #[test]
    fn home_hides_logo_when_connection_active() {
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
        let mut done = false;
        let mut h = Harness::new_ui(|ui| {
            if ready(ui, &mut done) {
                show(ui, &s);
            }
        });

        // Act
        h.run();

        // Assert
        assert!(h.query_by_label("cRustDB logo").is_none());
    }

    #[test]
    fn home_filter_by_label_hides_others() {
        // Arrange
        let mut s = state(&["alpha", "beta"]);
        s.config.connections[0].labels = vec!["dev".into()];
        s.config.connections[1].labels = vec!["prod".into()];
        s.config.labels = ["dev", "prod"]
            .map(|n| crate::storage::config::Label {
                name: n.into(),
                color: [1, 2, 3],
            })
            .to_vec();
        let mut done = false;
        let mut h = Harness::new_ui(|ui| {
            if ready(ui, &mut done) {
                show(ui, &s);
            }
        });
        h.ctx
            .data_mut(|d| d.insert_temp(labels::filter_id(), Some("dev".to_string())));

        // Act
        h.run();

        // Assert
        h.get_by_label("alpha");
        assert!(h.query_by_label("beta").is_none());
    }

    #[test]
    fn home_filter_resets_when_label_deleted() {
        // Arrange
        let s = state(&["alpha"]);
        let mut done = false;
        let mut h = Harness::new_ui(|ui| {
            if ready(ui, &mut done) {
                show(ui, &s);
            }
        });
        h.ctx
            .data_mut(|d| d.insert_temp(labels::filter_id(), Some("gone".to_string())));

        // Act
        h.run();

        // Assert
        h.get_by_label("alpha");
    }

    #[test]
    fn engine_colors_differ() {
        // Act & Assert
        assert_ne!(
            engine_color(DriverKind::Postgres),
            engine_color(DriverKind::Mysql)
        );
    }
}
