//! One frame of `CrustApp`: drain events, run commands, draw panels, apply intents.
use eframe::egui;

use super::CrustApp;
use crate::keymap::{Action, keymap};
use crate::runtime::{Command, Event};
use crate::ui::{self, Intent};

impl CrustApp {
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let now = std::time::Instant::now();
        while let Ok(e) = self.events.try_recv() {
            self.status.on_event(&e, &self.state, now);
            if let Some(f) = &mut self.form
                && f.on_event(&e)
                && let Event::Connected { ws, .. } | Event::ConnectFailed { ws, .. } = &e
                && let Some(rt) = &self.runtime
            {
                rt.send(Command::Disconnect { ws: *ws });
            }
            self.record_history(&e);
            self.state.apply(e);
        }
        self.apply_picked();
        let cmds: Vec<_> = self
            .state
            .workspaces
            .iter_mut()
            .filter(|w| matches!(w.status, crate::state::Status::Online(_)))
            .flat_map(|w| {
                let mut v = w.tree.load_for_filter();
                v.extend(w.tree.load_roots());
                v
            })
            .collect();
        for c in cmds {
            self.send(c);
        }
        if let Some(rt) = &self.runtime {
            for c in self.state.take_fetches(|| rt.next_request_id()) {
                rt.send(c);
            }
        }
        for cmd in self.state.take_commands() {
            self.send(cmd);
        }
        self.status.tick(now, || self.sampler.sample());
        ui.ctx()
            .request_repaint_after(crate::state::status::SAMPLE_EVERY);
        let mut intents = vec![];
        let focus = self.focus(ui.ctx());
        if let Some(a) = ui.input(|i| keymap(i, focus)) {
            if a == Action::RowDetail {
                ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space));
            }
            intents.extend(self.on_action(a, focus));
        }
        if let Some(m) = self.status.current(&self.state) {
            egui::Panel::bottom("status_bar").show(ui, |ui| ui::status_bar::show(ui, m));
        }
        self.sidebar_rect = None;
        if self.state.active_online().is_some() {
            let r = egui::Panel::left("sidebar").show(ui, |ui| {
                intents.extend(ui::sidebar::show(ui, &self.state));
            });
            self.sidebar_rect = Some(r.response.rect);
        }
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(f) = &mut self.form {
                intents.extend(ui::connection_form::show(ui, f));
                return;
            }
            if let Some(w) = self.state.active_online() {
                intents.extend(ui::tab_bar::show(ui, w));
            }
            let ws = self
                .state
                .active
                .and_then(|ws| self.state.workspace_mut(ws));
            if let Some(w) = ws
                && let Some(q) = w
                    .active_tab
                    .and_then(|t| w.queries.iter_mut().find(|q| q.id == t))
            {
                egui::Panel::bottom(("results", q.id.0))
                    .resizable(true)
                    .default_size(240.0)
                    .show(ui, |ui| {
                        intents.extend(ui::results::show(ui, q));
                    });
                if let Some(h) = &mut self.history {
                    egui::Panel::right("history")
                        .default_size(260.0)
                        .show(ui, |ui| {
                            if let Some(sql) = ui::history_panel::show(ui, h) {
                                intents.push(Intent::InsertHistory { tab: q.id, sql });
                            }
                        });
                }
                intents.extend(ui::editor::show(ui, q, &w.tree, &w.columns));
                return;
            }
            let tab = self
                .state
                .active
                .and_then(|ws| self.state.workspace_mut(ws))
                .and_then(|w| w.active_table_mut());
            match tab {
                Some(t) => intents.extend(ui::grid::show_tab(ui, t)),
                None => match self.state.active_online() {
                    Some(w) if w.config.database.is_some() => {
                        intents.extend(ui::workspace_home::show(ui, w));
                    }
                    _ => intents.extend(ui::home::show(ui, &self.state)),
                },
            }
        });
        intents.extend(ui::confirm_dialog::show(ui.ctx(), &mut self.state.dialogs));
        if let Some(w) = self
            .state
            .active
            .and_then(|ws| self.state.workspace_mut(ws))
        {
            ui::diagram::show(ui.ctx(), &mut w.diagram);
        }
        for i in intents {
            self.handle(i);
        }
        self.status.frame_done(std::time::Instant::now());
    }
}
