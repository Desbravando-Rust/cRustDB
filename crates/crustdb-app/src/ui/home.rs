use crustdb_core::model::DriverKind;
use eframe::egui;
use egui_phosphor::regular as icon;

use super::{Intent, labels};
use crate::state::home_filter::{UNGROUPED, filter_connections, group_by_label};
use crate::state::{AppState, Status};
use crate::storage::config::ConnectionConfig;

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

const BANNER_MIN: egui::Vec2 = egui::vec2(420.0, 430.0);
const WATERMARK_MIN: egui::Vec2 = egui::vec2(240.0, 300.0);

/// Memory id of the home search text; session-only.
fn search_id() -> egui::Id {
    egui::Id::unique("home_search")
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

/// Central-panel empty state: watermark, hint and book banner. The banner
/// yields first on small windows, then the watermark.
pub fn empty_state(ui: &mut egui::Ui, state: &AppState) {
    let (w, h) = (ui.available_width(), ui.available_height());
    if w >= BANNER_MIN.x && h >= BANNER_MIN.y {
        book_promo(ui);
    }
    let watermark = w >= WATERMARK_MIN.x && h >= WATERMARK_MIN.y;
    ui.vertical_centered(|ui| {
        let content = if watermark { 200.0 + 60.0 } else { 40.0 };
        ui.add_space(((ui.available_height() - content) / 2.0).max(0.0));
        if watermark {
            ui.add(
                egui::Image::new(egui::include_image!("../../../../assets/imgs/logo2.png"))
                    .fit_to_exact_size(egui::vec2(200.0, 200.0))
                    .tint(egui::Color32::WHITE.gamma_multiply(0.1))
                    .alt_text("cRustDB watermark"),
            );
        }
        if state.config.connections.is_empty() {
            ui.label("Create a connection to get started");
        } else {
            ui.label("Select a connection");
            ui.weak("Double-click to connect · Right-click for options");
        }
    });
}

fn book_promo(ui: &mut egui::Ui) {
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
}

/// Saved connections: label filter, `New` and the list (sidebar content).
pub fn show(ui: &mut egui::Ui, state: &AppState) -> Vec<Intent> {
    let mut out = vec![];
    egui::ScrollArea::vertical().show(ui, |ui| {
        let mut filter = None;
        let mut query: String = ui.data(|d| d.get_temp(search_id())).unwrap_or_default();
        ui.add(
            egui::TextEdit::singleline(&mut query)
                .hint_text("Search name, host, label...")
                .desired_width(ui.available_width()),
        );
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
        ui.data_mut(|d| d.insert_temp(search_id(), query.clone()));
        ui.separator();
        if state.config.connections.is_empty() {
            ui.weak("No saved connections. Create one with New.");
        }
        let shown = filter_connections(&state.config.connections, &query, filter.as_deref());
        for (name, items) in group_by_label(&shown, &state.config.labels) {
            let title = name.as_deref().unwrap_or(UNGROUPED);
            egui::CollapsingHeader::new(title)
                .id_salt(("home_group", title))
                .default_open(true)
                .show(ui, |ui| {
                    for c in items {
                        row(ui, state, c, &mut out);
                    }
                });
        }
    });
    out
}

/// One connection line: color stripe, engine, name, address, chip and actions.
fn row(ui: &mut egui::Ui, state: &AppState, c: &ConnectionConfig, out: &mut Vec<Intent>) {
    let ws = state.workspaces.iter().rev().find(|w| w.config.id == c.id);
    let live = state
        .live_workspace(c.id)
        .and_then(|id| state.workspaces.iter().find(|w| w.id == id));
    let connecting = live.is_some_and(|w| w.status == Status::Connecting);
    let top = ui.vertical(|ui| {
        ui.horizontal(|ui| {
            let engine = egui::RichText::new(icon::DATABASE).color(engine_color(c.driver));
            ui.label(engine).on_hover_text(engine_name(c.driver));
            let name = egui::RichText::new(&c.name).strong();
            if ui.selectable_label(false, name).double_clicked() && !connecting {
                out.push(Intent::Connect(c.id));
            }
            if let Some(n) = &c.label {
                labels::chip(ui, n, &state.config.labels);
            }
        });
        ui.label(
            egui::RichText::new(format!("{}@{}:{}", c.user, c.host, c.port))
                .color(egui::Color32::GRAY),
        );
        // Same visual order as #20: Active/Open or Connect, edit, delete.
        ui.horizontal(|ui| {
            if let Some(w) = live.filter(|w| matches!(w.status, Status::Online(_))) {
                ui.colored_label(ACTIVE, "Active");
                if ui.button("Open").clicked() {
                    out.push(Intent::SwitchWorkspace(w.id));
                }
            } else {
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
            }
            if ui
                .button(icon::PENCIL_SIMPLE)
                .on_hover_text("Edit")
                .clicked()
            {
                out.push(Intent::EditConnection(c.id));
            }
            if ui
                .add_enabled(!state.config.read_only, egui::Button::new(icon::TRASH))
                .on_hover_text("Delete")
                .clicked()
            {
                out.push(Intent::DeleteConnection(c.id));
            }
        });
    });
    if let Some(n) = &c.label {
        let r = top.response.rect;
        let stripe = egui::Rect::from_min_max(
            egui::pos2(r.min.x - 6.0, r.min.y),
            egui::pos2(r.min.x - 3.0, r.max.y),
        );
        let color = labels::color32(labels::color_of(&state.config.labels, n));
        ui.painter().rect_filled(stripe, 1.0, color);
    }
    if let Some(Status::Failed(e)) = ws.map(|w| &w.status) {
        ui.colored_label(ui.visuals().error_fg_color, e);
    }
}

#[cfg(test)]
#[path = "home_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "home_empty_tests.rs"]
mod empty_tests;
