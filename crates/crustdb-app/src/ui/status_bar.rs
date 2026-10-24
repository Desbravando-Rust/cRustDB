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
    ui.label(RichText::new(text).color(color))
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
        if let Some(mode) = m.tls {
            let (i, c) = tls_icon(mode);
            item(ui, i.into(), c, &format!("TLS mode: {mode:?}"));
        }
        if m.ssh {
            item(ui, icon::TERMINAL_WINDOW.into(), GREEN, "SSH tunnel");
        }
        match m.tx {
            TxState::Idle => {}
            TxState::Open => item(ui, "TX open".into(), AMBER, "Transaction open"),
            TxState::Aborted => item(ui, "TX aborted".into(), RED, "Transaction aborted"),
        }
        item(
            ui,
            format!("{} {}", icon::SHIELD_CHECK, m.threshold),
            GRAY,
            &safety_tooltip(m.threshold),
        );
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
        if let Some(p) = m.proc {
            item(
                ui,
                format!("{} {:.0}%", icon::CPU, p.cpu_percent),
                GRAY,
                "Process CPU (100% = one core, summed over threads)",
            );
            let mb = p.rss_bytes / (1024 * 1024);
            item(
                ui,
                format!("{} {mb} MB", icon::MEMORY),
                GRAY,
                "Resident memory",
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

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
    fn tooltip_contains_safety_rules() {
        // Arrange
        let t = safety_tooltip(1000);

        // Act & Assert
        for needle in ["1000", "WHERE", "Password", "Confirm", "DROP"] {
            assert!(t.contains(needle), "missing {needle}: {t}");
        }
    }
}
