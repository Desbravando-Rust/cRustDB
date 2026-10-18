//! Tab, focus and shortcut handling for `CrustApp`.
use eframe::egui;

use super::CrustApp;
use crate::keymap::{Action, Focus};
use crate::runtime::{Command, TabId};
use crate::ui::Intent;

impl CrustApp {
    /// Applies `f` to the table tab and refetches its page if `f` reports a change.
    pub(super) fn table_op(
        &mut self,
        tab: TabId,
        f: impl FnOnce(&mut crate::state::TableTab) -> bool,
    ) {
        let (Some(rt), Some(t)) = (&self.runtime, self.state.table_mut(tab)) else {
            return;
        };
        if f(t) {
            rt.send(t.fetch(rt.next_request_id()));
        }
    }

    pub(super) fn active_ws(&mut self) -> Option<&mut crate::state::WorkspaceState> {
        let id = self.state.active?;
        self.state.workspace_mut(id)
    }

    /// Sends the command; a `CloseTab` also drops the tab from the strip.
    pub(super) fn send_close(&mut self, cmd: Option<Command>) {
        if let Some(Command::CloseTab { tab, .. }) = &cmd {
            self.state
                .workspaces
                .iter_mut()
                .for_each(|w| w.remove_tab(*tab));
        }
        self.send(cmd);
    }

    pub(super) fn focus(&mut self, ctx: &egui::Context) -> Focus {
        if ctx.input(|i| i.pointer.primary_pressed())
            && let Some(p) = ctx.input(|i| i.pointer.interact_pos())
        {
            self.sidebar_focus = self.sidebar_rect.is_some_and(|r| r.contains(p));
        }
        let typing = ctx.text_edit_focused();
        let Some(w) = self.active_ws() else {
            return Focus::Other;
        };
        let (query, table) = (
            w.active_query_mut().is_some(),
            w.active_table_mut().is_some(),
        );
        match (self.sidebar_focus, typing, query, table) {
            (true, ..) => Focus::Other,
            (_, true, true, _) => Focus::Editor,
            (_, false, _, true) => Focus::Grid,
            _ => Focus::Other,
        }
    }

    /// Actions with state-side logic run here; Run/RunAll and Ctrl+/ belong to the editor,
    /// the rest of §4.9 has no feature behind it yet.
    pub(super) fn on_action(&mut self, a: Action, focus: Focus) -> Vec<Intent> {
        if a == Action::NewConnection {
            return vec![Intent::NewConnection];
        }
        let Some((tab, sel)) = self
            .active_ws()
            .map(|w| (w.active_tab, w.tree.selected.clone()))
        else {
            return vec![];
        };
        match (a, tab) {
            (Action::NewQuery, _) => vec![Intent::NewQuery],
            (Action::CloseTab, Some(t)) => vec![Intent::CloseTab(t)],
            (Action::NextTab | Action::PrevTab, _) => {
                let delta = if a == Action::NextTab { 1 } else { -1 };
                self.active_ws().iter_mut().for_each(|w| w.cycle_tab(delta));
                vec![]
            }
            (Action::Cancel, Some(t)) => vec![Intent::StopTab(t)],
            (Action::Refresh, Some(t)) if focus == Focus::Grid => {
                self.table_op(t, |_| true);
                vec![]
            }
            (Action::Refresh, _) => vec![match sel {
                Some(sel) => Intent::RefreshNode(sel),
                None => Intent::RefreshRoot,
            }],
            (Action::Filter, _) => {
                if let Some(t) = self.active_ws().and_then(|w| w.active_table_mut()) {
                    t.toggle_filter_bar();
                }
                vec![]
            }
            (Action::RowDetail, _) => {
                if let Some(t) = self.active_ws().and_then(|w| w.active_table_mut()) {
                    t.toggle_detail();
                }
                vec![]
            }
            (Action::NextPage, Some(t)) => vec![Intent::NextPage(t)],
            (Action::PrevPage, Some(t)) => vec![Intent::PrevPage(t)],
            _ => vec![],
        }
    }
}
