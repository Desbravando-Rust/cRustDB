//! Autocomplete popup: key capture before the `TextEdit`, and the list under the cursor.
use eframe::egui::{self, Key, Modifiers, text::CCursor, text::CCursorRange};

use crate::state::autocomplete::{Autocomplete, AutocompleteAction};

const POPUP: &str = "autocomplete-popup";

/// Arrows, Enter, Tab and Esc go to the popup, not to the text, while it is open and the editor
/// (`id`) has focus; otherwise the popup closes and the keys stay with whoever has focus.
pub fn take_key(ctx: &egui::Context, id: egui::Id, ac: &mut Autocomplete) -> AutocompleteAction {
    if ac.open && !ctx.memory(|m| m.has_focus(id)) && !pointer_on_popup(ctx) {
        ac.close();
    }
    if !ac.open {
        return AutocompleteAction::None;
    }
    for key in [
        Key::ArrowUp,
        Key::ArrowDown,
        Key::Enter,
        Key::Tab,
        Key::Escape,
    ] {
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key)) {
            let a = ac.on_key(key);
            if a != AutocompleteAction::None {
                return a;
            }
        }
    }
    AutocompleteAction::None
}

/// Replaces `range` (bytes) of `text` and returns the cursor (chars) after the new text.
pub fn accept(text: &mut String, item: &str, range: std::ops::Range<usize>) -> usize {
    let start = range.start.min(text.len());
    let end = range.end.clamp(start, text.len());
    text.replace_range(start..end, item);
    text[..start].chars().count() + item.chars().count()
}

pub fn set_cursor(ctx: &egui::Context, id: egui::Id, chars: usize) {
    if let Some(mut st) = egui::TextEdit::load_state(ctx, id) {
        st.cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(chars))));
        st.store(ctx, id);
    }
}

/// Draws the list at `pos`; returns the index the user clicked.
pub fn popup(ctx: &egui::Context, pos: egui::Pos2, ac: &Autocomplete) -> Option<usize> {
    let mut clicked = None;
    egui::Area::new(egui::Id::NULL.with(POPUP))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_height(200.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, item) in ac.items.iter().enumerate() {
                        let r = ui.selectable_label(i == ac.selected, item);
                        if i == ac.selected {
                            r.scroll_to_me(None);
                        }
                        if r.clicked() {
                            clicked = Some(i);
                        }
                    }
                });
            });
        });
    clicked
}

/// A click on a list item first takes focus from the editor; that must not close the popup.
fn pointer_on_popup(ctx: &egui::Context) -> bool {
    let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::NULL.with(POPUP));
    ctx.pointer_latest_pos()
        .is_some_and(|p| ctx.layer_id_at(p) == Some(layer))
}
