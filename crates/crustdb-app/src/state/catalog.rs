//! Schema changes made from a query tab (DDL): refresh everything that cached the catalog.
use super::autocomplete::ColumnCache;
use super::workspace::{AppState, Tab};
use crate::runtime::{Command, RequestId, TabId};

impl AppState {
    /// The sidebar tree, the home list, the column cache and the structure of every table tab
    /// without staged edits are stale: reload them.
    pub(super) fn catalog_changed(&mut self, tab: TabId) {
        let Some(w) = self
            .workspaces
            .iter_mut()
            .find(|w| w.queries.iter().any(|q| q.id == tab))
        else {
            return;
        };
        let mut cmds = w.tree.reload_loaded();
        cmds.extend(w.expand_home());
        w.columns = ColumnCache::default();
        for Tab::Table(t) in &mut w.tabs {
            // A tab with staged edits keeps its columns; it reloads on the next manual refresh.
            t.needs_structure = !t.has_pending();
        }
        self.outbox.extend(cmds);
    }

    /// Commands for tabs that need their catalog structure reloaded or a page fetched (the
    /// first one after the structure arrived).
    pub fn take_fetches(&mut self, mut next_req: impl FnMut() -> RequestId) -> Vec<Command> {
        self.workspaces
            .iter_mut()
            .flat_map(|w| w.tabs.iter_mut())
            .map(|Tab::Table(t)| t)
            .filter_map(|t| {
                if std::mem::take(&mut t.needs_structure) {
                    Some(t.load_structure(next_req()))
                } else {
                    std::mem::take(&mut t.needs_fetch).then(|| t.fetch(next_req()))
                }
            })
            .collect()
    }
}
