//! Bottom status bar (§4.8): icons and colours only, data comes from `StatusModel`.
use std::time::Duration;

use crustdb_core::driver::TlsMode;
use crustdb_core::model::{Product, ServerVersion};
use eframe::egui::{self, Color32, RichText};
use egui_phosphor::regular as icon;

use crate::runtime::TxState;
use crate::state::status::StatusModel;

const GREEN: Color32 = Color32::from_rgb(90, 190, 110);
const AMBER: Color32 = Color32::from_rgb(230, 170, 60);
const RED: Color32 = Color32::from_rgb(220, 90, 90);
const GRAY: Color32 = Color32::GRAY;

/// Closed lock = verified, amber = `require`/`verify-ca`, open and red = `disable`.
pub fn tls_icon(mode: TlsMode) -> (&'static str, Color32) {
    match mode {
        TlsMode::VerifyFull => (icon::LOCK, GREEN),
        TlsMode::Require | TlsMode::VerifyCa => (icon::LOCK, AMBER),
        TlsMode::Disable => (icon::LOCK_OPEN, RED),
    }
}

pub fn safety_tooltip(threshold: u64) -> String {
    format!(
        "Safety rules (threshold {threshold} rows)\n\
         UPDATE/DELETE without WHERE: Confirm\n\
         DML affecting more than 1 row: Confirm\n\
         DML over {threshold} rows or unknown impact: Password\n\
         DROP, TRUNCATE, ALTER and other destructive statements: Confirm"
    )
}

/// Plain text on the accent bar (state colours would vanish on orange).
fn item(ui: &mut egui::Ui, text: String, tip: &str) {
    ui.label(RichText::new(text).color(Color32::WHITE))
        .on_hover_text(tip);
}

fn ms(d: Duration) -> String {
    format!("{} ms", d.as_millis())
}

fn product_name(p: Product) -> &'static str {
    match p {
        Product::Postgres => "PostgreSQL",
        Product::Mysql => "MySQL",
        Product::Mariadb => "MariaDB",
        Product::Sqlite => "SQLite",
        Product::Redis => "Redis",
    }
}

/// `PostgreSQL 15.4`
fn engine(v: &ServerVersion) -> String {
    format!("{} {}", product_name(v.product), v.raw)
}

fn rows_text(m: &StatusModel) -> Option<String> {
    match (m.grid_rows, m.rows) {
        (Some((n, Some(total))), _) => Some(format!("Rows: {n} of {}", total.max(n))),
        (Some((n, None)), _) | (None, Some(n)) => Some(format!("Rows: {n}")),
        (None, None) => None,
    }
}

fn mode_text(m: &StatusModel) -> (String, &'static str) {
    match &m.connection {
        Some((_, _, false)) => (format!("{} Reconnecting", icon::PLUGS), "Reconnecting"),
        Some(_) if m.ssh => (
            format!("{} Connected via SSH Tunnel", icon::TERMINAL_WINDOW),
            "SSH tunnel",
        ),
        Some(_) => (format!("{} Connected", icon::PLUGS_CONNECTED), "Connected"),
        None => (format!("{} Not connected", icon::PLUGS), "Not connected"),
    }
}

pub fn show(ui: &mut egui::Ui, m: &StatusModel) {
    ui.horizontal(|ui| {
        if let Some(scope) = &m.scope {
            item(
                ui,
                format!("{} {scope}", icon::GIT_BRANCH),
                "Connection and active database or schema",
            );
        }
        if let Some((_, v, _)) = &m.connection {
            item(
                ui,
                format!("{} {}", icon::DATABASE, engine(v)),
                "Database engine and version",
            );
        }
        match m.tx {
            TxState::Idle => {}
            TxState::Open => item(ui, "TX open".into(), "Transaction open"),
            TxState::Aborted => item(ui, "TX aborted".into(), "Transaction aborted"),
        }
        for (_, rows) in &m.exports {
            item(
                ui,
                format!("{} {rows} rows", icon::DOWNLOAD_SIMPLE),
                "CSV export running",
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (text, tip) = mode_text(m);
            item(ui, text, tip);
            if let Some(q) = m.last_query {
                let tip = format!(
                    "First row: {}\nFetch total: {}\nRender: {}",
                    ms(q.first_row),
                    ms(q.total),
                    ms(q.render)
                );
                item(ui, format!("Query time: {}", ms(q.total)), &tip);
            }
            if let Some(l) = m.latency {
                item(ui, format!("{} {}", icon::PULSE, ms(l)), "Ping latency");
            }
            if let Some(t) = rows_text(m) {
                item(
                    ui,
                    t,
                    "Rows on screen of the table estimate, or rows returned or affected",
                );
            }
        });
    });
}

/// Metrics row under the grid footer: engine version, TLS, safety rules, process CPU and memory.
pub fn metrics(ui: &mut egui::Ui, m: &StatusModel) {
    ui.horizontal(|ui| {
        let put = |ui: &mut egui::Ui, text: String, color: Color32, tip: &str| {
            ui.label(RichText::new(text).color(color))
                .on_hover_text(tip);
        };
        if let Some((_, v, up)) = &m.connection {
            let c = if *up { GREEN } else { AMBER };
            put(
                ui,
                format!("{} {}", icon::CIRCLE, v.raw),
                c,
                &format!("Server: {}", engine(v)),
            );
        }
        if let Some(mode) = m.tls {
            let (i, c) = tls_icon(mode);
            put(ui, i.into(), c, &format!("TLS mode: {mode:?}"));
        }
        put(
            ui,
            format!("{} {}", icon::SHIELD_CHECK, m.threshold),
            GRAY,
            &safety_tooltip(m.threshold),
        );
        if let Some(p) = m.proc {
            put(
                ui,
                format!("{} {:.0}%", icon::CPU, p.cpu_percent),
                GRAY,
                "Process CPU (100% = one core, summed over threads)",
            );
            let mb = p.rss_bytes / (1024 * 1024);
            put(
                ui,
                format!("{} {mb} MB", icon::MEMORY),
                GRAY,
                "Resident memory",
            );
        }
    });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    /// Panel fill behind the metrics row.
    const PANEL: Color32 = Color32::from_rgb(22, 26, 36);
    use egui_kittest::kittest::Queryable;

    #[test]
    fn status_icons_follow_tls_mode() {
        // Arrange
        let (closed, ok) = tls_icon(TlsMode::VerifyFull);

        // Act & Assert
        let (amber_icon, amber) = tls_icon(TlsMode::Require);
        assert_eq!((amber_icon, amber), tls_icon(TlsMode::VerifyCa));
        let (cut, bad) = tls_icon(TlsMode::Disable);
        assert_eq!((closed, amber_icon), (icon::LOCK, icon::LOCK));
        assert_eq!(cut, icon::LOCK_OPEN);
        assert!(ok != amber && amber != bad && ok != bad);
    }

    #[test]
    fn state_colours_are_legible_on_panel() {
        // Arrange
        let luma = |c: Color32| {
            f32::from(c.r()) * 0.299 + f32::from(c.g()) * 0.587 + f32::from(c.b()) * 0.114
        };

        // Act & Assert
        for c in [GREEN, AMBER, RED, GRAY] {
            assert!(luma(c) - luma(PANEL) > 80.0, "{c:?}");
        }
    }

    #[test]
    fn tooltip_contains_safety_rules() {
        // Arrange
        let t = safety_tooltip(1000);

        // Act & Assert
        for needle in ["1000", "WHERE", "Password", "Confirm", "DROP"] {
            assert!(t.contains(needle), "missing {needle}: {t}");
        }
    }

    fn model() -> StatusModel {
        let version = crustdb_core::model::ServerVersion {
            product: crustdb_core::model::Product::Postgres,
            major: 16,
            minor: 1,
            patch: 0,
            raw: "16.1".into(),
        };
        let mut m = StatusModel::default();
        m.connection = Some(("pg".into(), version, true));
        m.tls = Some(TlsMode::VerifyFull);
        m.ssh = true;
        m.threshold = 1000;
        m.rows = Some(7);
        m.grid_rows = Some((7, Some(15_244)));
        m.scope = Some("pg/main".into());
        m.last_query = Some(crate::state::status::QueryTiming {
            first_row: Duration::from_millis(5),
            total: Duration::from_millis(12),
            render: Duration::ZERO,
        });
        m.latency = Some(Duration::from_millis(3));
        m.proc = Some(crate::metrics::ProcStats {
            cpu_percent: 12.0,
            rss_bytes: 64 * 1024 * 1024,
        });
        m
    }

    fn harness<'a>(m: &'a StatusModel, strip: bool) -> egui_kittest::Harness<'a> {
        egui_kittest::Harness::new_ui(move |ui| {
            crate::ui::theme::install_theme(ui.ctx());
            if strip {
                metrics(ui, m);
            } else {
                show(ui, m);
            }
        })
    }

    #[test]
    fn metrics_strip_shows_tls_safety_cpu_memory() {
        // Arrange
        let m = model();
        let mut h = harness(&m, true);

        // Act
        h.run();

        // Assert
        for l in [
            icon::LOCK.to_string(),
            format!("{} 16.1", icon::CIRCLE),
            format!("{} 1000", icon::SHIELD_CHECK),
            format!("{} 12%", icon::CPU),
            format!("{} 64 MB", icon::MEMORY),
        ] {
            h.get_by_label(&l);
        }
    }

    #[test]
    fn status_bar_keeps_version_rows_time_ssh_and_omits_moved_items() {
        // Arrange
        let m = model();
        let mut h = harness(&m, false);

        // Act
        h.run();

        // Assert
        h.get_by_label(&format!("{} pg/main", icon::GIT_BRANCH));
        h.get_by_label_contains("PostgreSQL 16.1");
        h.get_by_label(&format!(
            "{} Connected via SSH Tunnel",
            icon::TERMINAL_WINDOW
        ));
        h.get_by_label("Rows: 7 of 15244");
        h.get_by_label("Query time: 12 ms");
        for gone in [icon::LOCK, icon::SHIELD_CHECK, icon::CPU, icon::MEMORY] {
            assert!(h.query_by_label_contains(gone).is_none(), "{gone}");
        }
    }
}
