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
            let p = &mut tab.pane;
            if ui
                .add(
                    egui::TextEdit::singleline(&mut p.filter)
                        .hint_text("Filter rows")
                        .desired_width(160.0),
                )
                .changed()
            {
                p.refresh_view();
            }
            if p.filter.is_empty() {
                ui.weak(format!("{} rows", p.rows.len()));
            } else {
                ui.weak(format!("{} of {} rows", p.view.len(), p.rows.len()));
            }
        }
        if tab.pane.load_more_visible()
            && ui
                .add_enabled(tab.can_run(), egui::Button::new("Load more"))
                .clicked()
        {
            out.push(Intent::LoadMore(id));
        }
        if tab.pane.header.is_some()
            && ui
                .add_enabled(tab.can_run(), egui::Button::new("Export CSV..."))
                .clicked()
        {
            out.push(Intent::ExportResult(id));
        }
    });
    ui.separator();
    match tab.pane.tab {
        ResultTab::Result => {
            let p = &mut tab.pane;
            let clicked = match &p.header {
                Some(h) => grid::grid(
                    ui,
                    &h.columns,
                    &p.rows,
                    Some(p.view.as_slice()),
                    &mut p.selected,
                    &mut p.select_anchor,
                    &p.sorts,
                ),
                None => {
                    ui.weak("No result");
                    None
                }
            };
            if let Some((column, shift)) = clicked {
                p.click_header(&column, shift);
            }
        }
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
    use crustdb_core::model::{Cell, ColumnMeta, DriverKind, TypeCategory};
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
        // Arrange
        let mut tab = QueryTab::new(TabId(1), DriverKind::Postgres, None);

        // Act & Assert
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
        // Arrange
        let mut tab = QueryTab::new(TabId(1), DriverKind::Postgres, None);
        tab.pane.header = Some(ResultHeader { columns: vec![] });

        // Act & Assert
        assert!(run(&mut tab, show).query_by_label("Load more").is_some());
        tab.pane.exhausted = true;
        assert!(run(&mut tab, show).query_by_label("Load more").is_none());
    }

    fn tab_with_rows() -> QueryTab {
        let col = |n: &str| ColumnMeta {
            name: n.into(),
            type_name: "text".into(),
            category: TypeCategory::Text,
            nullable: None,
            is_primary_key: false,
        };
        let text = |s: &str| Cell::Text {
            value: s.into(),
            truncated: false,
        };
        let mut tab = QueryTab::new(TabId(1), DriverKind::Postgres, None);
        tab.pane.header = Some(ResultHeader {
            columns: vec![col("id"), col("name")],
        });
        tab.pane.rows = [("1", "a"), ("2", "b"), ("3", "c")]
            .into_iter()
            .map(|(i, n)| vec![text(i), text(n)].into())
            .collect();
        tab.pane.refresh_view();
        tab
    }

    fn copied(h: &Harness<'_>) -> Vec<String> {
        h.output()
            .platform_output
            .commands
            .iter()
            .filter_map(|c| match c {
                egui::OutputCommand::CopyText(t) => Some(t.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn copy_event_copies_selected_row_as_tsv() {
        // Arrange
        let mut tab = tab_with_rows();
        tab.pane.selected = Some(1);
        tab.pane.select_anchor = Some(1);
        let mut h = run(&mut tab, show);
        h.event(egui::Event::Copy);

        // Act
        h.step();

        // Assert
        assert_eq!(copied(&h), ["2\tb"]);
    }

    #[test]
    fn copy_follows_display_order_of_sorted_view() {
        // Arrange
        let mut tab = tab_with_rows();
        tab.pane.view = vec![2, 1, 0];
        tab.pane.select_anchor = Some(2);
        tab.pane.selected = Some(1);
        let mut h = run(&mut tab, show);
        h.event(egui::Event::Copy);

        // Act
        h.step();

        // Assert
        assert_eq!(copied(&h), ["3\tc\n2\tb"]);
    }

    #[test]
    fn shift_click_extends_selection_to_copy_two_rows() {
        // Arrange
        let mut tab = tab_with_rows();
        let mut h = run(&mut tab, show);
        h.get_by_label("1").click();
        h.run();
        h.get_by_label("2").click_modifiers(egui::Modifiers::SHIFT);
        h.run();
        h.event(egui::Event::Copy);

        // Act
        h.step();

        // Assert
        assert_eq!(copied(&h), ["1\ta\n2\tb"]);
    }

    #[test]
    fn context_menu_copies_raw_cell() {
        // Arrange
        let mut tab = tab_with_rows();
        let mut h = run(&mut tab, show);
        h.get_by_label("b").click_secondary();
        h.run();
        h.get_by_label("Copy cell").click();

        // Act
        h.step();

        // Assert
        assert_eq!(copied(&h), ["b"]);
    }
}
