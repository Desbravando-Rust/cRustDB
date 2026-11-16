//! Menu bar (File, Edit, View, Tools, Help): every item maps to an existing action or intent.
use eframe::egui;

use super::Intent;
use crate::keymap::{self, Action};
use crate::state::AppState;

const README_URL: &str = "https://github.com/Desbravando-Rust/cRustDB#readme";

/// What a clicked item asks the app to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuCmd {
    /// Same path as the keyboard shortcut.
    Action(Action),
    Intent(Intent),
}

struct Item<'a> {
    ui: &'a mut egui::Ui,
    out: &'a mut Vec<MenuCmd>,
}

impl Item<'_> {
    fn action(&mut self, label: &str, a: Action, enabled: bool) {
        if self.entry(label, keymap::shortcut(a), enabled) {
            self.out.push(MenuCmd::Action(a));
        }
    }

    fn intent(&mut self, label: &str, i: Intent, enabled: bool) {
        if self.entry(label, None, enabled) {
            self.out.push(MenuCmd::Intent(i));
        }
    }

    fn entry(&mut self, label: &str, sc: Option<egui::KeyboardShortcut>, enabled: bool) -> bool {
        self.entry_hint(label, sc, enabled, None)
    }

    fn entry_hint(
        &mut self,
        label: &str,
        sc: Option<egui::KeyboardShortcut>,
        enabled: bool,
        why_off: Option<String>,
    ) -> bool {
        let mut b = egui::Button::new(label);
        if let Some(sc) = sc {
            b = b.shortcut_text(self.ui.ctx().format_shortcut(&sc));
        }
        let mut r = self.ui.add_enabled(enabled, b);
        if let Some(w) = why_off {
            r = r.on_disabled_hover_text(w);
        }
        if r.clicked() {
            self.ui.close();
        }
        r.clicked()
    }
}

pub fn show(ui: &mut egui::Ui, state: &AppState) -> Vec<MenuCmd> {
    let mut out = vec![];
    let online = state.active_online();
    let on = online.is_some();
    let tab = online.is_some_and(|w| w.active_tab.is_some());
    let table = online.is_some_and(|w| w.tabs.iter().any(|t| Some(t.id) == w.active_tab));
    let sql = online.is_some_and(|w| w.capabilities().sql_editor);
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| {
            let mut m = Item { ui, out: &mut out };
            m.action("New connection…", Action::NewConnection, true);
            let why = online.map(|w| super::home::unsupported_for(w.config.driver));
            if m.entry_hint(
                "New query",
                keymap::shortcut(Action::NewQuery),
                sql,
                why.filter(|_| !sql),
            ) {
                m.out.push(MenuCmd::Action(Action::NewQuery));
            }
            m.action("Open file…", Action::Open, on);
            m.action("Save file", Action::Save, tab);
            m.action("Save file as…", Action::SaveAs, tab);
            m.action("Export CSV…", Action::ExportCsv, tab);
            m.action("Close tab", Action::CloseTab, tab);
            if let Some(w) = online {
                m.intent("Disconnect", Intent::Disconnect(w.id), true);
            } else {
                m.entry("Disconnect", None, false);
            }
            if m.entry("Quit", None, true) {
                m.ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
        ui.menu_button("Edit", |ui| {
            let mut m = Item { ui, out: &mut out };
            m.action("Find / filter", Action::Filter, on);
        });
        ui.menu_button("View", |ui| {
            let mut m = Item { ui, out: &mut out };
            m.action("Refresh data", Action::Refresh, on);
            m.action("Next tab", Action::NextTab, tab);
            m.action("Previous tab", Action::PrevTab, tab);
            m.action("Next page", Action::NextPage, table);
            m.action("Previous page", Action::PrevPage, table);
        });
        ui.menu_button("Tools", |ui| {
            let mut m = Item { ui, out: &mut out };
            m.action("Query history", Action::History, on);
            m.action("Cancel query", Action::Cancel, tab);
        });
        ui.menu_button("Help", |ui| {
            let mut m = Item { ui, out: &mut out };
            if m.entry("Releases", None, true) {
                m.ui.ctx()
                    .open_url(egui::OpenUrl::new_tab(crate::news::RELEASES_URL));
            }
            if m.entry("README", None, true) {
                m.ui.ctx().open_url(egui::OpenUrl::new_tab(README_URL));
            }
        });
    });
    out
}

#[cfg(test)]
#[path = "menu_bar_tests.rs"]
mod tests;
