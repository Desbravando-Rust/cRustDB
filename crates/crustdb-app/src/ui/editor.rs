//! SQL editor: highlighted multiline `TextEdit`, line-number gutter and run shortcuts.
use std::ops::Range;

use crustdb_core::model::DriverKind;
use crustdb_core::sql::{
    Expect, StatementSpan, TokenKind, completion_context, highlight, split, statement_at,
};
use eframe::egui::{
    self, Color32, FontId, Key, Modifiers, TextFormat, cache::ComputerMut, text::LayoutJob,
};

use super::Intent;
use super::autocomplete as popup;
use crate::state::QueryTab;
use crate::state::autocomplete::{AutocompleteAction, ColumnCache, in_literal_or_comment};
use crate::state::query_tab::{char_to_byte, highlight_enabled};
use crate::state::tree::ObjectTree;

type Spans = Vec<(Range<usize>, TokenKind)>;

#[derive(Default)]
struct Highlighter;

// `DriverKind` is not `Hash`, so the cache key carries a flag instead.
impl ComputerMut<(&str, bool), Spans> for Highlighter {
    fn compute(&mut self, (text, mysql): (&str, bool)) -> Spans {
        highlight(text, kind(mysql))
    }
}

type HighlightCache = egui::cache::FrameCache<Spans, Highlighter>;

#[derive(Default)]
struct Splitter;

impl ComputerMut<(&str, bool), Vec<StatementSpan>> for Splitter {
    fn compute(&mut self, (text, mysql): (&str, bool)) -> Vec<StatementSpan> {
        split(text, kind(mysql))
    }
}

type SplitCache = egui::cache::FrameCache<Vec<StatementSpan>, Splitter>;

fn kind(mysql: bool) -> DriverKind {
    if mysql {
        DriverKind::Mysql
    } else {
        DriverKind::Postgres
    }
}

fn color(k: TokenKind) -> Color32 {
    match k {
        TokenKind::Keyword => Color32::from_rgb(197, 134, 192),
        TokenKind::String => Color32::from_rgb(206, 145, 120),
        TokenKind::Number => Color32::from_rgb(181, 206, 168),
        TokenKind::Comment => Color32::from_rgb(106, 153, 85),
        TokenKind::QuotedIdent => Color32::from_rgb(156, 220, 254),
        TokenKind::Operator | TokenKind::Punct => Color32::from_rgb(212, 212, 212),
        TokenKind::Ident | TokenKind::Whitespace => Color32::from_rgb(220, 220, 220),
    }
}

/// Splits each token at the current-statement boundaries so that range gets a background.
fn job(
    text: &str,
    spans: Option<&Spans>,
    current: Option<Range<usize>>,
    font: FontId,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    let tint = Color32::from_rgb(40, 48, 62);
    let mut push = |range: Range<usize>, c: Color32| {
        let cur = current.clone().unwrap_or(0..0);
        let (a, b) = (range.start.max(cur.start), range.end.min(cur.end));
        let mut piece = |r: Range<usize>, bg: Color32| {
            if r.start < r.end {
                let f = TextFormat {
                    font_id: font.clone(),
                    color: c,
                    background: bg,
                    ..Default::default()
                };
                job.append(&text[r], 0.0, f);
            }
        };
        if a < b {
            piece(range.start..a, Color32::TRANSPARENT);
            piece(a..b, tint);
            piece(b..range.end, Color32::TRANSPARENT);
        } else {
            piece(range, Color32::TRANSPARENT);
        }
    };
    match spans {
        Some(s) => s.iter().for_each(|(r, k)| push(r.clone(), color(*k))),
        None => push(0..text.len(), color(TokenKind::Ident)),
    }
    job
}

pub fn show(
    ui: &mut egui::Ui,
    tab: &mut QueryTab,
    tree: &ObjectTree,
    cols: &ColumnCache,
) -> Vec<Intent> {
    let mut out = vec![];
    let id = ui.make_persistent_id(("query-editor", tab.id.0));
    let sel = egui::TextEdit::load_state(ui.ctx(), id)
        .and_then(|s| s.cursor.char_range())
        .map(|r| {
            let [a, b] = r.sorted_cursors();
            usize::from(a.index)..usize::from(b.index)
        })
        .unwrap_or(0..0);
    let run_all =
        ui.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Enter));
    let run = !run_all && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter));
    if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Slash)) {
        tab.toggle_comment_at(sel.clone());
    }
    if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::F)) {
        tab.format_at(sel.clone());
    }
    // Wraps so a narrow window does not push the editor and results off-screen.
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(tab.can_run(), egui::Button::new("Run (Ctrl+Enter)"))
            .clicked()
        {
            out.push(Intent::RunQuery {
                tab: tab.id,
                run: sel.clone(),
                all: false,
            });
        }
        if ui
            .add_enabled(
                tab.can_run(),
                egui::Button::new("Run all (Ctrl+Shift+Enter)"),
            )
            .clicked()
        {
            out.push(Intent::RunQuery {
                tab: tab.id,
                run: sel.clone(),
                all: true,
            });
        }
        if ui.button("Format (Ctrl+Shift+F)").clicked() {
            tab.format_at(sel.clone());
        }
        out.extend(files_bar(ui, tab));
        out.extend(super::results::toolbar(ui, tab));
    });
    if run || run_all {
        tab.ac.close();
        out.push(Intent::RunQuery {
            tab: tab.id,
            run: sel.clone(),
            all: run_all,
        });
    }
    let ctx = ui.ctx().clone();
    let mut moved = None;
    if let AutocompleteAction::Accept(item, range) = popup::take_key(&ctx, id, &mut tab.ac) {
        moved = Some(popup::accept(&mut tab.text, &item, range));
        tab.dirty = true;
    }
    if let Some(sql) = tab.insert.take() {
        moved = Some(tab.insert_at(sel.end, &sql));
    }
    if let Some(c) = moved {
        popup::set_cursor(&ctx, id, c);
    }
    let font = FontId::monospace(14.0);
    let driver = tab.driver;
    let cursor = sel.end;
    if !highlight_enabled(&tab.text) {
        ui.weak("Highlighting disabled for large files (1 MiB or 750 lines)");
    }
    let lines = tab.text.split('\n').count();
    let err_line = tab.error_marker.as_ref().map(|(l, _)| *l);
    egui::ScrollArea::both().show(ui, |ui| {
        ui.horizontal_top(|ui| {
            let mut g = LayoutJob::default();
            for n in 0..lines {
                let bad = err_line == Some(n);
                g.append(
                    &format!("{}{:>4} \n", if bad { "●" } else { " " }, n + 1),
                    0.0,
                    TextFormat {
                        font_id: font.clone(),
                        color: if bad {
                            Color32::LIGHT_RED
                        } else {
                            Color32::GRAY
                        },
                        ..Default::default()
                    },
                );
            }
            ui.label(g);
            let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, _w: f32| {
                let text = buf.as_str();
                let j = if highlight_enabled(text) {
                    // Derived from the text the layouter receives: it may already be edited this frame.
                    let key = (text, driver == DriverKind::Mysql);
                    let byte = char_to_byte(text, cursor);
                    let (current, spans) = ui.ctx().memory_mut(|m| {
                        let cur = statement_at(m.caches.cache::<SplitCache>().get(key), byte)
                            .map(|s| s.range.clone());
                        (cur, m.caches.cache::<HighlightCache>().get(key).clone())
                    });
                    job(text, Some(&spans), current, font.clone())
                } else {
                    job(text, None, None, font.clone())
                };
                ui.fonts_mut(|f| f.layout_job(j))
            };
            let edit = egui::TextEdit::multiline(&mut tab.text)
                .id(id)
                .code_editor()
                .hint_text("SQL")
                .desired_width(f32::INFINITY)
                .desired_rows(24)
                .layouter(&mut layouter)
                .show(ui);
            let r = &edit.response;
            if r.changed() {
                tab.dirty = true;
                tab.error_marker = None;
            }
            let Some(cur) = edit.cursor_range.map(|c| c.primary) else {
                return;
            };
            let byte = char_to_byte(&tab.text, usize::from(cur.index));
            let force = r.has_focus()
                && ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Space));
            if r.changed() || force {
                if in_literal_or_comment(&tab.text, byte, driver) {
                    tab.ac.close();
                } else {
                    let cx = completion_context(&tab.text, byte, driver);
                    tab.ac.refresh(&cx, tree, cols);
                    if matches!(cx.expect, Expect::Column { .. }) {
                        out.extend(cols.wanted(&cx, tree).into_iter().map(Intent::LoadColumns));
                    }
                }
            } else if r.clicked() || !(tab.ac.replace.start..=tab.ac.replace.end).contains(&byte) {
                tab.ac.close();
            }
            if tab.ac.open {
                let pos =
                    edit.galley_pos + edit.galley.pos_from_cursor(cur).left_bottom().to_vec2();
                if let Some(i) = popup::popup(&ctx, pos, &tab.ac) {
                    let range = tab.ac.replace.clone();
                    let c = popup::accept(&mut tab.text, &tab.ac.items[i].clone(), range);
                    tab.ac.close();
                    tab.dirty = true;
                    popup::set_cursor(&ctx, id, c);
                }
            }
        });
    });
    out
}

/// Open / Save / Save as / History.
fn files_bar(ui: &mut egui::Ui, tab: &QueryTab) -> Vec<Intent> {
    let mut out = vec![];
    ui.separator();
    ui.label(tab.title());
    for (label, intent) in [
        ("Open", Intent::OpenFile),
        ("Save", Intent::Save(tab.id)),
        ("Save as", Intent::SaveAs(tab.id)),
        ("History", Intent::ToggleHistory),
    ] {
        if ui.button(label).clicked() {
            out.push(intent);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;
    use crate::runtime::TabId;

    #[test]
    fn editor_draws_highlighted_text_with_error_gutter() {
        let mut tab = QueryTab::new(TabId(1), DriverKind::Postgres, None);
        tab.text = "select 'a', 1; -- c\nselect 2".into();
        tab.error_marker = Some((1, 20..21));
        let (tree, cols) = (
            ObjectTree::new(crate::runtime::WorkspaceId(1), DriverKind::Postgres),
            ColumnCache::default(),
        );
        let mut ready = false;
        let mut h = Harness::new_ui(|ui| {
            if !std::mem::replace(&mut ready, true) {
                crate::ui::theme::install_theme(ui.ctx());
                return;
            }
            show(ui, &mut tab, &tree, &cols);
        });
        h.run();
        h.run();
    }

    #[test]
    fn typing_multibyte_inside_statement_does_not_panic() {
        use egui_kittest::kittest::Queryable;
        let mut tab = QueryTab::new(TabId(1), DriverKind::Postgres, None);
        tab.text = "select 'x'".into();
        let (tree, cols) = (
            ObjectTree::new(crate::runtime::WorkspaceId(1), DriverKind::Postgres),
            ColumnCache::default(),
        );
        let mut ready = false;
        let mut h = Harness::new_ui(|ui| {
            if !std::mem::replace(&mut ready, true) {
                crate::ui::theme::install_theme(ui.ctx());
                return;
            }
            show(ui, &mut tab, &tree, &cols);
        });
        h.run();
        h.get_by_role(egui::accesskit::Role::MultilineTextInput)
            .focus();
        h.run();
        // Cursor before the closing quote, so the edit lands inside the statement.
        h.key_press(egui::Key::ArrowLeft);
        h.run();
        h.get_by_role(egui::accesskit::Role::MultilineTextInput)
            .type_text("é");
        h.run();
        h.run();
        drop(h);
        assert_eq!(tab.text, "select 'xé'");
    }

    #[test]
    fn enter_accepts_the_suggestion_instead_of_a_newline() {
        use egui_kittest::kittest::Queryable;
        let mut tab = QueryTab::new(TabId(1), DriverKind::Postgres, None);
        let (tree, cols) = (
            ObjectTree::new(crate::runtime::WorkspaceId(1), DriverKind::Postgres),
            ColumnCache::default(),
        );
        let mut ready = false;
        let mut h = Harness::new_ui(|ui| {
            if !std::mem::replace(&mut ready, true) {
                crate::ui::theme::install_theme(ui.ctx());
                return;
            }
            show(ui, &mut tab, &tree, &cols);
        });
        h.run();
        let role = egui::accesskit::Role::MultilineTextInput;
        h.get_by_role(role).focus();
        h.run();
        h.get_by_role(role).type_text("sele");
        h.run();
        h.key_press(egui::Key::Enter);
        h.run();
        drop(h);
        assert!(!tab.text.contains('\n'), "{:?}", tab.text);
        assert!(tab.text.to_lowercase().starts_with("sele"));
        assert!(tab.text.len() > 4, "{:?}", tab.text);
        assert!(!tab.ac.open);
    }

    /// Types `typed` into a fresh editor holding `text`, popup open afterwards.
    fn typed(
        text: &str,
        typed: &str,
    ) -> (Harness<'static>, std::rc::Rc<std::cell::RefCell<QueryTab>>) {
        let tab = std::rc::Rc::new(std::cell::RefCell::new(QueryTab::new(
            TabId(1),
            DriverKind::Postgres,
            None,
        )));
        tab.borrow_mut().text = text.into();
        let t = tab.clone();
        let (tree, cols) = (
            ObjectTree::new(crate::runtime::WorkspaceId(1), DriverKind::Postgres),
            ColumnCache::default(),
        );
        let mut ready = false;
        let mut h = Harness::new_ui(move |ui| {
            if !std::mem::replace(&mut ready, true) {
                crate::ui::theme::install_theme(ui.ctx());
                return;
            }
            show(ui, &mut t.borrow_mut(), &tree, &cols);
        });
        h.run();
        let role = egui::accesskit::Role::MultilineTextInput;
        h.get_by_role(role).focus();
        h.run();
        h.key_press(egui::Key::End);
        h.run();
        h.get_by_role(role).type_text(typed);
        h.run();
        (h, tab)
    }

    #[test]
    fn ctrl_space_reopens_the_popup() {
        let (mut h, tab) = typed("x ", "s");
        tab.borrow_mut().ac.close();
        h.run();
        assert!(!tab.borrow().ac.open);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Space);
        h.run();
        assert!(tab.borrow().ac.open);
    }

    #[test]
    fn popup_closes_when_cursor_leaves_the_prefix() {
        let (mut h, tab) = typed("x ", "s");
        assert!(tab.borrow().ac.open);
        h.key_press(egui::Key::Home);
        h.run();
        assert!(!tab.borrow().ac.open);
    }

    #[test]
    fn popup_closes_and_keeps_its_keys_when_editor_loses_focus() {
        let (mut h, tab) = typed("x ", "s");
        assert!(tab.borrow().ac.open);
        h.ctx.memory_mut(|m| {
            if let Some(f) = m.focused() {
                m.surrender_focus(f);
            }
        });
        h.run();
        let before = tab.borrow().text.clone();
        h.key_press(egui::Key::Enter);
        h.run();
        assert!(!tab.borrow().ac.open);
        assert_eq!(
            tab.borrow().text,
            before,
            "Enter was not taken by the popup"
        );
    }
}
