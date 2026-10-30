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

pub(crate) fn logo(ui: &mut egui::Ui) {
    ui.add(
        egui::Image::new(egui::include_image!("../../../../assets/imgs/logo2.png"))
            .fit_to_exact_size(egui::vec2(150.0, 150.0))
            .alt_text("cRustDB logo"),
    );
    ui.add_space(8.0);
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
                    .fit_to_exact_size(egui::vec2(80.0, 122.0))
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
            logo(ui);
        }
        let mut filter = None;
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
        });
        if let Some(w) = &state.config.warning {
            ui.colored_label(ui.visuals().warn_fg_color, w);
        }
        ui.separator();
        if state.config.connections.is_empty() {
            ui.weak("No saved connections. Create one with New.");
        }
        let shown = crate::state::home_filter::filter_connections(
            &state.config.connections,
            "",
            filter.as_deref(),
        );
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
                ui.label(
                    egui::RichText::new(format!("{}@{}:{}", c.user, c.host, c.port))
                        .color(egui::Color32::GRAY),
                );
                if let Some(n) = &c.label {
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
#[path = "home_tests.rs"]
pub(crate) mod tests;
