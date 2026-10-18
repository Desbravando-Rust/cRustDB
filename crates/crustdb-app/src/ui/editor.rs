//! SQL editor: highlighted multiline `TextEdit`, line-number gutter and run shortcuts.
use std::ops::Range;

use crustdb_core::model::DriverKind;
use crustdb_core::sql::{StatementSpan, TokenKind, highlight, split, statement_at};
use eframe::egui::{
    self, Color32, FontId, Key, Modifiers, TextFormat, cache::ComputerMut, text::LayoutJob,
};

use super::Intent;
use crate::state::QueryTab;
use crate::state::query_tab::{char_to_byte, highlight_enabled};

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

pub fn show(ui: &mut egui::Ui, tab: &mut QueryTab) -> Vec<Intent> {
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
    ui.horizontal(|ui| {
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
        out.extend(super::results::toolbar(ui, tab));
    });
    if run || run_all {
        out.push(Intent::RunQuery {
            tab: tab.id,
            run: sel.clone(),
            all: run_all,
        });
    }
    let font = FontId::monospace(14.0);
    let driver = tab.driver;
    let cursor = sel.end;
    if !highlight_enabled(&tab.text) {
        ui.weak("Highlighting disabled above 1 MiB");
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
            let r = ui.add(
                egui::TextEdit::multiline(&mut tab.text)
                    .id(id)
                    .code_editor()
                    .hint_text("SQL")
                    .desired_width(f32::INFINITY)
                    .desired_rows(24)
                    .layouter(&mut layouter),
            );
            if r.changed() {
                tab.dirty = true;
                tab.error_marker = None;
            }
        });
    });
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use egui_kittest::Harness;

    use super::*;
    use crate::runtime::TabId;

    #[test]
    fn editor_draws_highlighted_text_with_error_gutter() {
        let mut tab = QueryTab::new(TabId(1), DriverKind::Postgres, None);
        tab.text = "select 'a', 1; -- c\nselect 2".into();
        tab.error_marker = Some((1, 20..21));
        let mut ready = false;
        let mut h = Harness::new_ui(|ui| {
            if !std::mem::replace(&mut ready, true) {
                crate::ui::theme::install_theme(ui.ctx());
                return;
            }
            show(ui, &mut tab);
        });
        h.run();
        h.run();
    }

    #[test]
    fn typing_multibyte_inside_statement_does_not_panic() {
        use egui_kittest::kittest::Queryable;
        let mut tab = QueryTab::new(TabId(1), DriverKind::Postgres, None);
        tab.text = "select 'x'".into();
        let mut ready = false;
        let mut h = Harness::new_ui(|ui| {
            if !std::mem::replace(&mut ready, true) {
                crate::ui::theme::install_theme(ui.ctx());
                return;
            }
            show(ui, &mut tab);
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
}
