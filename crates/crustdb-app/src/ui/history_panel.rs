//! Query history of the connection: search box and a list; double click inserts the SQL.
use eframe::egui;

use crate::storage::history::{HistoryEntry, HistoryStore};

#[derive(Default)]
pub struct HistoryPanel {
    /// Newest first.
    pub entries: Vec<HistoryEntry>,
    pub filter: String,
}

/// The SQL of the entry that was double clicked, if any.
pub fn show(ui: &mut egui::Ui, panel: &mut HistoryPanel) -> Option<String> {
    ui.heading("History");
    ui.add(egui::TextEdit::singleline(&mut panel.filter).hint_text("search"));
    let mut picked = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        for e in HistoryStore::search(&panel.entries, &panel.filter) {
            let first = e.sql.lines().next().unwrap_or_default();
            let mark = if e.ok { "" } else { "! " };
            let r = ui
                .selectable_label(false, format!("{mark}{first}"))
                .on_hover_text(format!("{} ({} ms)\n{}", e.ts, e.duration_ms, e.sql));
            if r.double_clicked() {
                picked = Some(e.sql.clone());
            }
        }
    });
    picked
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;

    fn entry(sql: &str) -> HistoryEntry {
        HistoryEntry {
            ts: "2026-01-01T00:00:00Z".into(),
            database: None,
            sql: sql.into(),
            duration_ms: 1,
            rows: None,
            ok: true,
            error: None,
        }
    }

    #[test]
    fn double_click_returns_the_entry_sql() {
        // Arrange
        let mut panel = HistoryPanel {
            entries: vec![entry("select 2"), entry("select 1")],
            filter: String::new(),
        };
        let mut picked = None;
        let mut h = Harness::builder().with_step_dt(0.05).build_ui(|ui| {
            picked = picked.take().or(show(ui, &mut panel));
        });
        h.run();
        h.get_by_label("select 1").click();
        h.step();
        h.get_by_label("select 1").click();
        h.step();

        // Act
        h.run();
        drop(h);

        // Assert
        assert_eq!(picked.as_deref(), Some("select 1"));
    }
}
