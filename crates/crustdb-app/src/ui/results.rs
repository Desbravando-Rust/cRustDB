//! Query tab output: toolbar (run/stop, transaction, database) and the Result / Messages panes.
use crustdb_core::sql::TxOp;
use eframe::egui::{self, Color32, RichText};

use super::{Intent, grid};
use crate::runtime::TxState;
use crate::state::QueryTab;
use crate::state::query_tab::ResultTab;

/// Buttons appended to the editor's toolbar row.
pub fn toolbar(ui: &mut egui::Ui, tab: &mut QueryTab) -> Vec<Intent> {
    let mut out = vec![];
    let id = tab.id;
    if ui
        .add_enabled(tab.can_stop(), egui::Button::new("Stop"))
        .clicked()
    {
        out.push(Intent::StopTab(id));
    }
    ui.separator();
    for (op, label) in [
        (TxOp::Begin, "Begin"),
        (TxOp::Commit, "Commit"),
        (TxOp::Rollback, "Rollback"),
    ] {
        if ui.button(label).clicked() {
            out.push(Intent::Tx { tab: id, op });
        }
    }
    match tab.tx {
        TxState::Idle => ui.weak("Idle"),
        TxState::Open => ui.colored_label(Color32::LIGHT_GREEN, "Transaction open"),
        TxState::Aborted => ui.colored_label(Color32::LIGHT_RED, "Rollback required"),
    };
    ui.separator();
    ui.add_enabled_ui(tab.can_switch_db(), |ui| {
        let current = tab.database.as_deref().unwrap_or("");
        let r = ui.add(
            egui::TextEdit::singleline(&mut tab.db_draft)
                .hint_text(format!("Database: {current}"))
                .desired_width(120.0),
        );
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("Use").clicked() || enter) && !tab.db_draft.is_empty() {
            out.push(Intent::UseDatabase {
                tab: id,
                database: std::mem::take(&mut tab.db_draft),
            });
        }
    });
    out
}

pub fn show(ui: &mut egui::Ui, tab: &mut QueryTab) -> Vec<Intent> {
    let mut out = vec![];
    let id = tab.id;
    ui.horizontal(|ui| {
        ui.selectable_value(&mut tab.pane.tab, ResultTab::Result, "Result");
        ui.selectable_value(&mut tab.pane.tab, ResultTab::Messages, "Messages");
        if tab.pane.header.is_some() {
            ui.weak(format!("{} rows", tab.pane.rows.len()));
        }
        if tab.pane.load_more_visible()
            && ui
                .add_enabled(tab.can_run(), egui::Button::new("Load more"))
                .clicked()
        {
            out.push(Intent::LoadMore(id));
        }
        if tab.pane.header.is_some() {
            // ponytail: plain path field; native file dialog (rfd) is not in the dependency tree.
            ui.add(
                egui::TextEdit::singleline(&mut tab.export_path)
                    .hint_text("Export CSV to path")
                    .desired_width(200.0),
            );
            if ui
                .add_enabled(
                    tab.can_run() && !tab.export_path.trim().is_empty(),
                    egui::Button::new("Export"),
                )
                .clicked()
            {
                out.push(Intent::ExportResult(id));
            }
        }
    });
    ui.separator();
    match tab.pane.tab {
        ResultTab::Result => match &tab.pane.header {
            Some(h) => {
                grid::grid(ui, &h.columns, &tab.pane.rows, &mut tab.pane.selected);
            }
            None => {
                ui.weak("No result");
            }
        },
        ResultTab::Messages => {
            egui::ScrollArea::vertical().show(ui, |ui| {
                for m in &tab.pane.messages {
                    let text = format!(
                        "#{} {} ({} ms)",
                        m.index + 1,
                        m.text,
                        m.duration.as_millis()
                    );
                    if m.ok {
                        ui.label(text);
                    } else {
                        ui.label(RichText::new(text).color(Color32::LIGHT_RED));
                    }
                }
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crustdb_core::driver::ResultHeader;
    use crustdb_core::model::DriverKind;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    use super::*;
    use crate::runtime::{RequestId, TabId};

    fn run(tab: &mut QueryTab, f: fn(&mut egui::Ui, &mut QueryTab) -> Vec<Intent>) -> Harness<'_> {
        let mut h = Harness::new_ui(move |ui| {
            f(ui, tab);
        });
        h.run();
        h
    }

    #[test]
    fn stop_enabled_only_while_running() {
        let mut tab = QueryTab::new(TabId(1), DriverKind::Postgres, None);
        assert!(
            run(&mut tab, toolbar)
                .get_by_label("Stop")
                .accesskit_node()
                .is_disabled()
        );
        tab.begin_run(RequestId(1));
        assert!(
            !run(&mut tab, toolbar)
                .get_by_label("Stop")
                .accesskit_node()
                .is_disabled()
        );
    }

    #[test]
    fn load_more_button_follows_exhausted() {
        let mut tab = QueryTab::new(TabId(1), DriverKind::Postgres, None);
        tab.pane.header = Some(ResultHeader { columns: vec![] });
        assert!(run(&mut tab, show).query_by_label("Load more").is_some());
        tab.pane.exhausted = true;
        assert!(run(&mut tab, show).query_by_label("Load more").is_none());
    }
}
