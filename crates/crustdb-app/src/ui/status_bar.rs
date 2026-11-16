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

/// Warning or error: bold plus a distinct icon, so the signal does not rely on colour.
fn alert(ui: &mut egui::Ui, text: String, tip: &str) {
    ui.label(RichText::new(text).color(Color32::WHITE).strong())
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
    // A table tab shows its own rows only; `m.rows` belongs to the last statement of another tab.
    match (m.on_table, m.grid_rows, m.rows) {
        (_, Some((n, Some(total))), _) => Some(format!("Rows: {n} of {}", total.max(n))),
        (_, Some((n, None)), _) | (false, None, Some(n)) => Some(format!("Rows: {n}")),
        _ => None,
    }
}

/// Text, tooltip and whether it is a warning (bold, own icon).
fn mode_text(m: &StatusModel) -> (String, &'static str, bool) {
    match &m.connection {
        Some((_, _, false)) => (
            format!("{} Reconnecting", icon::ARROWS_CLOCKWISE),
            "Reconnecting",
            true,
        ),
        Some(_) if m.ssh => (
            format!("{} Connected via SSH Tunnel", icon::TERMINAL_WINDOW),
            "SSH tunnel",
            false,
        ),
        Some(_) => (
            format!("{} Connected", icon::PLUGS_CONNECTED),
            "Connected",
            false,
        ),
        None => (
            format!("{} Disconnected", icon::PLUGS),
            "Not connected",
            true,
        ),
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
            TxState::Aborted => alert(
                ui,
                format!("{} TX aborted", icon::WARNING),
                "Transaction aborted",
            ),
        }
        for (_, rows) in &m.exports {
            item(
                ui,
                format!("{} {rows} rows", icon::DOWNLOAD_SIMPLE),
                "CSV export running",
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (text, tip, warn) = mode_text(m);
            if warn {
                alert(ui, text, tip);
            } else {
                item(ui, text, tip);
            }
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

/// Metrics row panel for the central area. Added before the tab content so it is the lowest
/// bottom panel: under the grid footer, as in the prototype. Only for an online connection; the
/// caller returns before this on the connection form.
pub fn metrics_panel(ui: &mut egui::Ui, m: &StatusModel, online: bool) {
    if online {
        egui::Panel::bottom("metrics_strip").show(ui, |ui| metrics(ui, m));
    }
}

/// Metrics row under the grid footer: engine version, TLS, safety rules, process CPU and memory.
pub fn metrics(ui: &mut egui::Ui, m: &StatusModel) {
    ui.horizontal(|ui| {
        let put = |ui: &mut egui::Ui, text: String, color: Color32, tip: &str| {
            ui.label(RichText::new(text).color(color))
                .on_hover_text(tip);
        };
        if let Some((_, v, up)) = &m.connection {
            // Different glyph when reconnecting: not colour-only.
            let (g, c) = if *up {
                (icon::CIRCLE, GREEN)
            } else {
                (icon::WARNING, AMBER)
            };
            put(
                ui,
                format!("{g} {}", v.raw),
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
            assert!(luma(c) - luma(crate::ui::theme::PANEL_FILL) > 80.0, "{c:?}");
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

    #[test]
    fn table_tab_never_shows_rows_of_another_tab() {
        // Arrange
        let mut m = StatusModel::default();
        m.rows = Some(99);

        // Act
        m.on_table = true;
        let on_table = rows_text(&m);
        m.on_table = false;

        // Assert
        assert_eq!(on_table, None);
        assert_eq!(rows_text(&m).as_deref(), Some("Rows: 99"));
    }

    fn panel_harness<'a>(m: &'a StatusModel, online: bool) -> egui_kittest::Harness<'a> {
        egui_kittest::Harness::new_ui(move |ui| {
            crate::ui::theme::install_theme(ui.ctx());
            egui::CentralPanel::default().show(ui, |ui| {
                metrics_panel(ui, m, online);
                egui::Panel::bottom("footer").show(ui, |ui| ui.label("grid footer"));
            });
        })
    }

    #[test]
    fn metrics_panel_sits_under_footer_and_hides_when_offline() {
        // Arrange
        let m = model();
        let mut shown = panel_harness(&m, true);
        let mut offline = panel_harness(&m, false);

        // Act
        shown.run();
        offline.run();

        // Assert
        let footer = shown.get_by_label("grid footer").rect();
        let metric = shown
            .get_by_label(&format!("{} 1000", icon::SHIELD_CHECK))
            .rect();
        assert!(metric.top() >= footer.bottom());
        assert!(
            offline
                .query_by_label_contains(icon::SHIELD_CHECK)
                .is_none()
        );
    }

    #[test]
    fn warnings_carry_a_distinct_icon() {
        // Arrange
        let mut m = model();
        m.connection = None;
        let mut h = harness(&m, false);

        // Act
        h.run();

        // Assert
        h.get_by_label(&format!("{} Disconnected", icon::PLUGS));
    }
}
