//! Bottom status bar (§4.8): icons and colours only, data comes from `StatusModel`.
use std::time::Duration;

use crustdb_core::driver::TlsMode;
use eframe::egui::{self, Color32, RichText};
use egui_phosphor::regular as icon;

use crate::runtime::TxState;
use crate::state::status::StatusModel;

const GREEN: Color32 = Color32::from_rgb(90, 190, 110);
const AMBER: Color32 = Color32::from_rgb(230, 170, 60);
const RED: Color32 = Color32::from_rgb(220, 90, 90);
const GRAY: Color32 = Color32::GRAY;
/// Dark chip behind every item: state colours would vanish on the orange bar.
const CHIP: Color32 = Color32::from_rgb(22, 26, 36);

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

fn item(ui: &mut egui::Ui, text: String, color: Color32, tip: &str) {
    egui::Frame::new()
        .fill(CHIP)
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| ui.label(RichText::new(text).color(color)))
        .inner
        .on_hover_text(tip);
}

fn ms(d: Duration) -> String {
    format!("{} ms", d.as_millis())
}

pub fn show(ui: &mut egui::Ui, m: &StatusModel) {
    ui.horizontal(|ui| {
        match &m.connection {
            Some((name, v, up)) => {
                let (c, tip) = if *up {
                    (GREEN, "Connected")
                } else {
                    (AMBER, "Reconnecting")
                };
                item(ui, format!("{} {name} {}", icon::CIRCLE, v.raw), c, tip);
            }
            None => item(
                ui,
                format!("{} not connected", icon::CIRCLE),
                GRAY,
                "Not connected",
            ),
        }
        if m.ssh {
            item(ui, icon::TERMINAL_WINDOW.into(), GREEN, "SSH tunnel");
        }
        match m.tx {
            TxState::Idle => {}
            TxState::Open => item(ui, "TX open".into(), AMBER, "Transaction open"),
            TxState::Aborted => item(ui, "TX aborted".into(), RED, "Transaction aborted"),
        }
        if let Some(q) = m.last_query {
            let tip = format!(
                "First row: {}\nFetch total: {}\nRender: {}",
                ms(q.first_row),
                ms(q.total),
                ms(q.render)
            );
            item(ui, format!("{} {}", icon::TIMER, ms(q.total)), GRAY, &tip);
        }
        if let Some(n) = m.rows {
            item(
                ui,
                format!("{} {n}", icon::ROWS),
                GRAY,
                "Rows returned or affected",
            );
        }
        if let Some(l) = m.latency {
            item(
                ui,
                format!("{} {}", icon::PULSE, ms(l)),
                GRAY,
                "Ping latency",
            );
        }
        for (_, rows) in &m.exports {
            item(
                ui,
                format!("{} {rows} rows", icon::DOWNLOAD_SIMPLE),
                AMBER,
                "CSV export running",
            );
        }
    });
}

/// Metrics strip above the status bar: TLS, safety rules, process CPU and memory.
pub fn metrics(ui: &mut egui::Ui, m: &StatusModel) {
    ui.horizontal(|ui| {
        let put = |ui: &mut egui::Ui, text: String, color: Color32, tip: &str| {
            ui.label(RichText::new(text).color(color))
                .on_hover_text(tip);
        };
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
    fn state_colours_are_legible_on_chip() {
        // Arrange
        let luma = |c: Color32| {
            f32::from(c.r()) * 0.299 + f32::from(c.g()) * 0.587 + f32::from(c.b()) * 0.114
        };

        // Act & Assert
        for c in [GREEN, AMBER, RED, GRAY] {
            assert!(luma(c) - luma(CHIP) > 80.0, "{c:?}");
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
        h.get_by_label_contains("16.1");
        h.get_by_label(icon::TERMINAL_WINDOW);
        h.get_by_label(&format!("{} 7", icon::ROWS));
        for gone in [icon::LOCK, icon::SHIELD_CHECK, icon::CPU, icon::MEMORY] {
            assert!(h.query_by_label_contains(gone).is_none(), "{gone}");
        }
    }
}
