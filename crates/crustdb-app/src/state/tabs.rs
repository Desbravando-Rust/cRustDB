//! Tab bar logic: one order (tables, then queries), cycling and closing.
use super::dialogs::Dialog;
use super::query_tab::{QueryTab, char_to_byte, toggle_comment};
use super::workspace::{AppState, WorkspaceState};
use crate::runtime::{Command, TabId, TxState, WorkspaceId};

impl WorkspaceState {
    pub fn tab_ids(&self) -> Vec<TabId> {
        let tables = self.tabs.iter().map(|t| t.id());
        tables.chain(self.queries.iter().map(|q| q.id)).collect()
    }

    /// Ctrl+Tab / Ctrl+Shift+Tab: wraps around.
    pub fn cycle_tab(&mut self, delta: isize) {
        let ids = self.tab_ids();
        if ids.is_empty() {
            return;
        }
        let at = ids.iter().position(|t| Some(*t) == self.active_tab);
        let next = at.map_or(0, |i| {
            (i as isize + delta).rem_euclid(ids.len() as isize) as usize
        });
        self.active_tab = Some(ids[next]);
    }

    /// Drops the tab locally; the neighbour on the left (or the first) takes over if it was active.
    pub fn remove_tab(&mut self, tab: TabId) {
        let ids = self.tab_ids();
        let Some(at) = ids.iter().position(|t| *t == tab) else {
            return;
        };
        self.tabs.retain(|t| t.id() != tab);
        self.queries.retain(|q| q.id != tab);
        if self.active_tab == Some(tab) {
            let rest = self.tab_ids();
            self.active_tab = rest.get(at.saturating_sub(1)).copied();
        }
    }
}

impl QueryTab {
    /// Ctrl+/: comments or uncomments the lines touched by the selection (chars).
    pub fn toggle_comment_at(&mut self, sel: std::ops::Range<usize>) {
        let line = |c: usize| {
            let end = char_to_byte(&self.text, c);
            self.text[..end].matches('\n').count()
        };
        let (a, b) = (line(sel.start), line(sel.end));
        self.text = toggle_comment(&self.text, a..b + 1);
        self.dirty = true;
    }
}

impl AppState {
    /// The one close path: a dirty query tab asks first (`Dialog::DirtyFile`), then an open
    /// transaction asks (`Dialog::CloseWithTx`); otherwise the returned command closes it.
    /// The caller removes the tab when sending `CloseTab`.
    pub fn close_tab(&mut self, ws: WorkspaceId, tab: TabId) -> Option<Command> {
        let w = self.workspace_mut(ws)?;
        if w.queries.iter().any(|q| q.id == tab && q.dirty) {
            self.dialogs.push(Dialog::DirtyFile { tab });
            return None;
        }
        self.close_tab_tx(tab)
    }

    /// Second step of closing: the transaction guard (also after Save or Discard).
    pub fn close_tab_tx(&mut self, tab: TabId) -> Option<Command> {
        let open = self
            .workspaces
            .iter()
            .flat_map(|w| &w.queries)
            .any(|q| q.id == tab && q.tx != TxState::Idle);
        if open {
            self.dialogs.push(Dialog::CloseWithTx { tab });
            return None;
        }
        Some(Command::CloseTab { tab, resolve: None })
    }

    /// Discard in the unsaved-changes dialog, then the transaction guard.
    pub fn discard_dirty_and_close(&mut self) -> Option<Command> {
        match self.dialogs.discard_dirty()? {
            Command::CloseTab { tab, .. } => self.close_tab_tx(tab),
            other => Some(other),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crustdb_core::model::{ObjectKind, ObjectRef};

    use crate::runtime::RequestId;
    use crate::state::workspace::{Tab, tests::state};

    use super::*;

    fn setup() -> (AppState, WorkspaceId) {
        let mut s = state(&["a"]);
        let ws = s.open_workspace(s.config.connections[0].clone());
        let obj = ObjectRef {
            database: "d".into(),
            schema: None,
            name: "t".into(),
            kind: ObjectKind::Table,
        };
        s.open_table(ws, obj, RequestId(1));
        s.open_query(ws, None);
        (s, ws)
    }

    #[test]
    fn cycles_across_table_and_query_tabs_and_wraps() {
        let (mut s, ws) = setup();
        let w = s.workspace_mut(ws).unwrap();
        assert_eq!(w.active_tab, Some(TabId(2)));
        w.cycle_tab(1);
        assert_eq!(w.active_tab, Some(TabId(1)));
        w.cycle_tab(-1);
        assert_eq!(w.active_tab, Some(TabId(2)));
    }

    #[test]
    fn dirty_tab_asks_save_discard_cancel_before_closing() {
        let (mut s, ws) = setup();
        assert!(matches!(
            s.close_tab(ws, TabId(2)),
            Some(Command::CloseTab { resolve: None, .. })
        ));
        s.query_mut(TabId(2)).unwrap().dirty = true;
        assert!(s.close_tab(ws, TabId(2)).is_none());
        assert_eq!(s.dialogs.top(), Some(&Dialog::DirtyFile { tab: TabId(2) }));
        assert!(s.dialogs.cancel().is_none(), "cancel keeps the tab");
        assert!(s.dialogs.top().is_none());
    }

    #[test]
    fn dirty_dialog_comes_before_the_tx_guard() {
        let (mut s, ws) = setup();
        let q = s.query_mut(TabId(2)).unwrap();
        q.dirty = true;
        q.tx = TxState::Open;
        assert!(s.close_tab(ws, TabId(2)).is_none());
        assert_eq!(s.dialogs.top(), Some(&Dialog::DirtyFile { tab: TabId(2) }));
        assert!(s.discard_dirty_and_close().is_none());
        assert_eq!(
            s.dialogs.top(),
            Some(&Dialog::CloseWithTx { tab: TabId(2) })
        );
    }

    #[test]
    fn close_with_open_tx_asks_first() {
        let (mut s, ws) = setup();
        s.workspace_mut(ws).unwrap().queries[0].tx = TxState::Open;
        assert!(s.close_tab(ws, TabId(2)).is_none());
        assert_eq!(
            s.dialogs.top(),
            Some(&Dialog::CloseWithTx { tab: TabId(2) })
        );
        let cmd = s.close_tab(ws, TabId(1));
        assert!(matches!(
            cmd,
            Some(Command::CloseTab {
                tab: TabId(1),
                resolve: None
            })
        ));
    }

    #[test]
    fn tx_changed_event_makes_close_ask_first() {
        let (mut s, ws) = setup();
        s.apply(crate::runtime::Event::TxChanged {
            tab: TabId(2),
            state: TxState::Open,
        });
        assert!(s.close_tab(ws, TabId(2)).is_none());
        assert_eq!(
            s.dialogs.top(),
            Some(&Dialog::CloseWithTx { tab: TabId(2) })
        );
    }

    #[test]
    fn close_with_aborted_tx_asks_first() {
        let (mut s, ws) = setup();
        s.workspace_mut(ws).unwrap().queries[0].tx = TxState::Aborted;
        assert!(s.close_tab(ws, TabId(2)).is_none());
        assert_eq!(
            s.dialogs.top(),
            Some(&Dialog::CloseWithTx { tab: TabId(2) })
        );
    }

    #[test]
    fn toggle_comment_at_covers_selected_lines() {
        let mut q = QueryTab::new(TabId(1), crustdb_core::model::DriverKind::Postgres, None);
        q.text = "a\nb\nc".into();
        q.toggle_comment_at(2..4);
        assert_eq!(q.text, "a\n-- b\n-- c");
        q.toggle_comment_at(2..9);
        assert_eq!(q.text, "a\nb\nc");
    }

    #[test]
    fn reopening_an_open_object_focuses_its_tab() {
        let (mut s, ws) = setup();
        let obj = s
            .workspace_mut(ws)
            .unwrap()
            .tabs
            .iter()
            .map(|Tab::Table(t)| t.object.clone())
            .next();
        assert!(s.open_table(ws, obj.unwrap(), RequestId(2)).is_empty());
        let w = s.workspace_mut(ws).unwrap();
        assert_eq!((w.tabs.len(), w.active_tab), (1, Some(TabId(1))));
    }

    #[test]
    fn removing_the_active_tab_activates_its_neighbour() {
        let (mut s, ws) = setup();
        let w = s.workspace_mut(ws).unwrap();
        w.remove_tab(TabId(2));
        assert_eq!(w.active_tab, Some(TabId(1)));
        w.remove_tab(TabId(1));
        assert_eq!(w.active_tab, None);
    }
}
