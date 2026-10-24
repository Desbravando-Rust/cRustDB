//! Find/replace bar above the query editor (Ctrl+F).
use std::ops::Range;

use eframe::egui::{self, Key, Modifiers, text::CCursor, text::CCursorRange};

use crate::state::QueryTab;
use crate::state::query_tab::highlight_enabled;

/// Draws the bar while `tab.find` is open and returns the match (chars) the editor highlights.
/// Enter = next, Shift+Enter = previous (focus stays here), Esc closes.
pub fn show(ui: &mut egui::Ui, tab: &mut QueryTab, editor: egui::Id) -> Option<Range<usize>> {
    if !tab.find.open {
        return None;
    }
    let ctx = ui.ctx().clone();
    let state = egui::TextEdit::load_state(&ctx, editor);
    let sel = state.as_ref().and_then(|s| s.cursor.char_range()).map(|r| {
        let [a, b] = r.sorted_cursors();
        usize::from(a.index)..usize::from(b.index)
    });
    // Same limit as highlighting: past it, matches are only scanned on demand.
    let big = !highlight_enabled(&tab.text);
    let fid = editor.with("find");
    // An unfocused editor collapses its selection, so the last landing is remembered here.
    let lid = fid.with("landed");
    let landed = ui.data(|d| d.get_temp::<Range<usize>>(lid));
    let had = ui.memory(|m| m.has_focus(fid));
    let shift = ui.input(|i| i.modifiers.shift);
    // Taken before any TextEdit sees it, so it never reaches the editor as a newline.
    let enter = had && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
    let (mut select, mut click) = (None, false);
    ui.horizontal(|ui| {
        let find = ui.add(
            egui::TextEdit::singleline(&mut tab.find.query)
                .id(fid)
                .hint_text("Find")
                .desired_width(200.0),
        );
        if std::mem::take(&mut tab.find.focus) {
            find.request_focus();
        }
        ui.checkbox(&mut tab.find.case_sensitive, "Aa")
            .on_hover_text("Match case");
        let prev_btn = ui.button("Prev").clicked();
        let next_btn = ui.button("Next").clicked();
        let (prev, next) = (prev_btn || (enter && shift), next_btn || (enter && !shift));
        if prev || next {
            click = prev_btn || next_btn;
            // The first jump lands on the current match unless it is already selected.
            let cur = tab.find.current_match(&tab.text);
            select = if cur.is_some() && cur != sel && cur != landed {
                cur
            } else {
                tab.find.step(&tab.text, next)
            };
        }
        if !big {
            ui.label(tab.find.count_label(&tab.text));
        }
        let replace = ui.add(
            egui::TextEdit::singleline(&mut tab.find.replacement)
                .hint_text("Replace with")
                .desired_width(200.0),
        );
        if ui.button("Replace").clicked() && tab.find.replace_current(&mut tab.text) {
            tab.dirty = true;
            click = true;
            select = tab.find.current_match(&tab.text);
        }
        if ui.button("Replace all").clicked() && tab.find.replace_all(&mut tab.text) > 0 {
            tab.dirty = true;
        }
        // Esc drops focus before widgets run, so "had focus" is last frame's: `lost_focus`.
        let esc = ui.input(|i| i.key_pressed(Key::Escape));
        if ui.button("Close").clicked() || (esc && (find.lost_focus() || replace.lost_focus())) {
            tab.find.open = false;
        }
    });
    if let Some(r) = select.clone() {
        ctx.data_mut(|d| d.insert_temp(lid, r));
    }
    if let (Some(r), Some(mut st)) = (select.clone(), state) {
        st.cursor.set_char_range(Some(CCursorRange::two(
            CCursor::new(r.start),
            CCursor::new(r.end),
        )));
        st.store(&ctx, editor);
        tab.find.scroll = true;
        if click {
            ctx.memory_mut(|m| m.request_focus(editor));
        }
    }
    if !tab.find.open {
        return None;
    }
    select.or_else(|| {
        if big {
            None
        } else {
            tab.find.current_match(&tab.text)
        }
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crustdb_core::model::DriverKind;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;
    use crate::runtime::TabId;

    #[test]
    fn find_bar_counts_matches_and_replaces_all() {
        // Arrange
        let mut tab = QueryTab::new(TabId(1), DriverKind::Postgres, None);
        tab.text = "select 1; SELECT 2".into();
        tab.find.open = true;
        tab.find.focus = true;
        tab.find.replacement = "x".into();
        let mut h = Harness::new_ui(|ui| {
            show(ui, &mut tab, egui::Id::unique("editor"));
        });
        h.run();
        h.event(egui::Event::Text("select".into()));
        h.run();
        h.get_by_label("1/2");
        h.get_by_label("Replace all").click();

        // Act
        h.run();
        drop(h);

        // Assert
        assert_eq!(tab.text, "x 1; x 2");
        assert!(tab.dirty);
    }

    #[test]
    fn enter_in_find_keeps_the_editor_text() {
        // Arrange
        let mut tab = QueryTab::new(TabId(1), DriverKind::Postgres, None);
        tab.text = "select 1; select 2".into();
        tab.find.open = true;
        tab.find.focus = true;
        let id = egui::Id::unique("editor");
        let mut h = Harness::new_ui(|ui| {
            show(ui, &mut tab, id);
            ui.add(
                egui::TextEdit::multiline(&mut tab.text)
                    .id(id)
                    .hint_text("SQL"),
            );
        });
        h.run();
        h.event(egui::Event::Text("select".into()));
        h.run();
        h.key_press(Key::Enter);
        h.run();
        h.key_press(Key::Enter);
        h.run();

        // Act
        let focus_kept = h.ctx.memory(|m| m.has_focus(id.with("find")));
        drop(h);

        // Assert
        assert_eq!(tab.text, "select 1; select 2");
        assert_eq!(tab.find.current, 1);
        assert!(focus_kept);
    }
}
