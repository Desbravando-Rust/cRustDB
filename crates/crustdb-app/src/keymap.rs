//! Fixed keyboard shortcuts (§4.9). Pure: maps an input snapshot to an `Action`.
use eframe::egui::{InputState, Key, Modifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    NewConnection,
    NewQuery,
    CloseTab,
    NextTab,
    PrevTab,
    Refresh,
    Filter,
    NextPage,
    PrevPage,
    RowDetail,
    Run,
    RunAll,
    Cancel,
    Autocomplete,
    ToggleComment,
    Open,
    Save,
    SaveAs,
    History,
    ExportCsv,
}

/// Where the keyboard is: only `Grid` makes bare Space mean "row detail".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Editor,
    Grid,
    Other,
}

const C: Modifiers = Modifiers::COMMAND;
const CS: Modifiers = Modifiers::COMMAND.plus(Modifiers::SHIFT);

const TABLE: &[(Modifiers, Key, Action)] = &[
    (C, Key::N, Action::NewConnection),
    (C, Key::T, Action::NewQuery),
    (C, Key::W, Action::CloseTab),
    (C, Key::Tab, Action::NextTab),
    (CS, Key::Tab, Action::PrevTab),
    (Modifiers::NONE, Key::F5, Action::Refresh),
    (C, Key::F, Action::Filter),
    (C, Key::CloseBracket, Action::NextPage),
    (C, Key::OpenBracket, Action::PrevPage),
    (C, Key::Enter, Action::Run),
    (CS, Key::Enter, Action::RunAll),
    (C, Key::Period, Action::Cancel),
    (C, Key::Space, Action::Autocomplete),
    (C, Key::Slash, Action::ToggleComment),
    (C, Key::O, Action::Open),
    (C, Key::S, Action::Save),
    (CS, Key::S, Action::SaveAs),
    (C, Key::H, Action::History),
    (CS, Key::E, Action::ExportCsv),
];

pub fn keymap(input: &InputState, focus: Focus) -> Option<Action> {
    if focus == Focus::Grid && input.modifiers.is_none() && input.key_pressed(Key::Space) {
        return Some(Action::RowDetail);
    }
    TABLE
        .iter()
        .find(|(m, k, _)| input.key_pressed(*k) && input.modifiers.matches_exact(*m))
        .map(|(_, _, a)| *a)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use eframe::egui::{self, Event, RawInput};

    use super::*;

    fn press(key: Key, modifiers: Modifiers, focus: Focus) -> Option<Action> {
        let ctx = egui::Context::default();
        let mut out = None;
        let raw = RawInput {
            events: vec![
                Event::ModifiersChanged(modifiers),
                Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed: true,
                    repeat: false,
                    modifiers,
                },
            ],
            ..Default::default()
        };
        let mut full = ctx.run_ui(raw, |ui| out = ui.input(|i| keymap(i, focus)));
        full.textures_delta.clear();
        out
    }

    #[test]
    fn every_spec_shortcut_maps_to_action() {
        use Action::*;
        let rows = [
            (C, Key::N, NewConnection),
            (C, Key::T, NewQuery),
            (C, Key::W, CloseTab),
            (C, Key::Tab, NextTab),
            (CS, Key::Tab, PrevTab),
            (Modifiers::NONE, Key::F5, Refresh),
            (C, Key::F, Filter),
            (C, Key::CloseBracket, NextPage),
            (C, Key::OpenBracket, PrevPage),
            (C, Key::Enter, Run),
            (CS, Key::Enter, RunAll),
            (C, Key::Period, Cancel),
            (C, Key::Space, Autocomplete),
            (C, Key::Slash, ToggleComment),
            (C, Key::O, Open),
            (C, Key::S, Save),
            (CS, Key::S, SaveAs),
            (C, Key::H, History),
            (CS, Key::E, ExportCsv),
        ];
        for (m, k, a) in rows {
            assert_eq!(press(k, m, Focus::Other), Some(a), "{k:?}");
        }
        assert_eq!(
            press(Key::Space, Modifiers::NONE, Focus::Grid),
            Some(RowDetail)
        );
    }

    #[test]
    fn space_toggles_row_detail_only_with_grid_focus() {
        assert_eq!(
            press(Key::Space, Modifiers::NONE, Focus::Grid),
            Some(Action::RowDetail)
        );
        for f in [Focus::Editor, Focus::Other] {
            assert_eq!(press(Key::Space, Modifiers::NONE, f), None);
        }
    }
}
