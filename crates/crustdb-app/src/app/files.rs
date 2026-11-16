//! Query files, native dialogs, history panel and column loading: the parts of `CrustApp`
//! added with the editor's file and autocomplete features.
use std::path::PathBuf;
use std::time::Duration;

use crustdb_core::model::ConnectionId;

use super::CrustApp;
use crate::runtime::{Command, Event, TabId};
use crate::state::Dialog;
use crate::state::connection_form::ConnectionForm;
use crate::ui::Intent;
use crate::ui::history_panel::HistoryPanel;
use crate::ui::picker::Pick;

impl CrustApp {
    pub(super) fn handle_files(&mut self, intent: Intent) {
        match intent {
            Intent::OpenFile => self.picker.start(Pick::Open, &self.ctx, ""),
            Intent::FormBrowseSqlite => {
                if let Some(f) = &self.form {
                    self.picker.start(Pick::SqliteDb(f.draft.id), &self.ctx, "");
                }
            }
            Intent::ExportResult(tab) => {
                self.picker
                    .start(Pick::Export(tab), &self.ctx, "result.csv")
            }
            Intent::ExportTable(tab) => {
                let name = self
                    .state
                    .table_mut(tab)
                    .map_or_else(|| "table.csv".into(), |t| export_file_name(&t.object.name));
                self.picker.start(Pick::ExportTable(tab), &self.ctx, &name)
            }
            Intent::Save(tab) => self.save(tab, false),
            Intent::SaveAs(tab) => self.save(tab, true),
            // The unsaved-changes dialog: save the tab it names, then close it.
            Intent::SaveFile => {
                if let Some(Dialog::DirtyFile { tab }) = self.state.dialogs.top().cloned() {
                    self.save(tab, false);
                }
            }
            Intent::ToggleHistory => self.toggle_history(),
            Intent::InsertHistory { tab, sql } => {
                if let Some(q) = self.state.query_mut(tab) {
                    q.insert = Some(sql);
                }
            }
            Intent::LoadColumns(object) => self.load_columns(object),
            Intent::OpenDiagram { database, schema } => {
                let Some(req) = self.runtime.as_ref().map(|rt| rt.next_request_id()) else {
                    return;
                };
                let cmd = self
                    .active_ws()
                    .and_then(|w| w.open_diagram(req, database, schema));
                self.send(cmd);
            }
            _ => {}
        }
    }

    pub(super) fn confirm_delete(&mut self) {
        let store = &self.store;
        let labels = self.state.config.labels.clone();
        match self.state.confirm_delete(|rest| store.save(rest, &labels)) {
            Ok(Some(id)) => {
                self.forget_secrets(id);
            }
            Ok(None) => {}
            Err(e) => {
                self.state.config.warning = Some(format!("could not save connections: {e}"));
            }
        }
    }

    fn save(&mut self, tab: TabId, as_new: bool) {
        let Some(q) = self.state.query_mut(tab) else {
            return;
        };
        match q.file.clone().filter(|_| !as_new) {
            Some(path) => self.write_file(tab, path),
            None => {
                let name = q.title().trim_end_matches('•').to_owned();
                self.picker
                    .start(Pick::Save(tab), &self.ctx, &format!("{name}.sql"));
            }
        }
    }

    fn write_file(&mut self, tab: TabId, path: PathBuf) {
        let Some(q) = self.state.query_mut(tab) else {
            return;
        };
        if let Err(e) = q.save_to(path) {
            note(q, format!("save failed: {e}"));
            return;
        }
        if self.state.dialogs.top() == Some(&Dialog::DirtyFile { tab }) {
            self.state.dialogs.pop();
            let cmd = self.state.close_tab_tx(tab);
            self.send_close(cmd);
        }
    }

    /// Results of the native dialogs started earlier.
    pub(super) fn apply_picked(&mut self) {
        for (pick, path) in self.picker.poll() {
            match pick {
                Pick::Save(tab) => self.write_file(tab, path),
                Pick::Export(tab) => {
                    let cmd = self.state.query_mut(tab).and_then(|q| q.export(path));
                    self.send(cmd);
                }
                Pick::ExportTable(tab) => {
                    let req = self.runtime.as_ref().map(|rt| rt.next_request_id());
                    let cmd = req.and_then(|r| self.state.table_mut(tab)?.export(path, r));
                    self.send(cmd);
                }
                Pick::SqliteDb(id) => apply_sqlite_pick(self.form.as_mut(), id, &path),
                Pick::Open => {
                    let Some(ws) = self.state.active else {
                        continue;
                    };
                    let cmd = self.state.open_query(ws, None);
                    self.send(cmd);
                    let q = self
                        .state
                        .workspace_mut(ws)
                        .and_then(|w| w.active_query_mut());
                    if let Some(q) = q
                        && let Err(e) = q.load_from(path)
                    {
                        note(q, format!("open failed: {e}"));
                    }
                }
            }
        }
    }

    fn toggle_history(&mut self) {
        if self.history.take().is_some() {
            return;
        }
        let Some(w) = self
            .state
            .active
            .and_then(|ws| self.state.workspace_mut(ws))
        else {
            return;
        };
        let mut entries = self
            .history_store
            .load(w.config.id)
            .inspect_err(|e| tracing::warn!("history load failed: {e}"))
            .unwrap_or_default();
        entries.reverse();
        self.history = Some(HistoryPanel {
            entries,
            filter: String::new(),
        });
    }

    /// Appends a finished statement to the connection's history file (and the open panel).
    pub(super) fn record_history(&mut self, e: &Event) {
        let (Event::StatementDone { req, history, .. } | Event::ExecError { req, history, .. }) = e
        else {
            return;
        };
        let conn = self
            .state
            .workspaces
            .iter()
            .find(|w| w.queries.iter().any(|q| q.pane.req == Some(*req)))
            .map(|w| w.config.id);
        let Some(conn) = conn else { return };
        if let Err(err) = self.history_store.append(conn, history) {
            tracing::warn!("history write failed: {err}");
        }
        if let Some(p) = &mut self.history {
            p.entries.insert(0, history.clone());
        }
    }

    /// Asks the workspace's Metadata session for a table's columns.
    fn load_columns(&mut self, object: crustdb_core::model::ObjectRef) {
        let Some(rt) = &self.runtime else { return };
        let req = rt.next_request_id();
        let Some(ws) = self.state.active else { return };
        let Some(w) = self.state.workspace_mut(ws) else {
            return;
        };
        if w.columns.asked(&object) {
            return;
        }
        w.columns.begin(object.clone(), req);
        self.send(Command::LoadColumns { ws, req, object });
    }
}

/// The dialog does not block: a form closed, swapped or switched off SQLite meanwhile drops the pick.
// ponytail: keyed by the draft's connection id, so reopening the same connection's edit
// form before the dialog closes still takes the pick; add a form generation if that matters.
fn apply_sqlite_pick(form: Option<&mut ConnectionForm>, id: ConnectionId, path: &std::path::Path) {
    if let Some(f) = form.filter(|f| f.draft.id == id && f.draft.driver.is_file()) {
        f.draft.database = Some(path.to_string_lossy().into_owned());
    }
}

fn note(q: &mut crate::state::QueryTab, text: String) {
    let n = q.pane.messages.len();
    q.pane.push(n, false, text, Duration::ZERO);
}

/// Suggested file name: the table name with characters that are invalid in file names replaced.
fn export_file_name(table: &str) -> String {
    let safe = table.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    format!("{safe}.csv")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crustdb_core::model::DriverKind;

    #[test]
    fn export_file_name_replaces_invalid_characters() {
        // Arrange
        let table = "a/b\\c:d*e?f\"g<h>i|j";

        // Act
        let name = export_file_name(table);

        // Assert
        assert_eq!(name, "a_b_c_d_e_f_g_h_i_j.csv");
        assert_eq!(export_file_name("users"), "users.csv");
    }

    fn sqlite_form() -> ConnectionForm {
        ConnectionForm::new(DriverKind::Sqlite, false)
    }

    #[test]
    fn sqlite_pick_fills_matching_form() {
        // Arrange
        let mut f = sqlite_form();
        let id = f.draft.id;

        // Act
        apply_sqlite_pick(Some(&mut f), id, std::path::Path::new("/tmp/a.db"));

        // Assert
        assert_eq!(f.draft.database.as_deref(), Some("/tmp/a.db"));
    }

    #[test]
    fn sqlite_pick_ignored_after_driver_change() {
        // Arrange
        let mut f = sqlite_form();
        let id = f.draft.id;
        f.set_driver(DriverKind::Postgres);
        let before = f.draft.database.clone();

        // Act
        apply_sqlite_pick(Some(&mut f), id, std::path::Path::new("/tmp/a.db"));

        // Assert
        assert_eq!(f.draft.database, before);
    }

    #[test]
    fn sqlite_pick_ignored_for_other_form() {
        // Arrange
        let mut f = sqlite_form();
        let other = sqlite_form().draft.id;

        // Act
        apply_sqlite_pick(Some(&mut f), other, std::path::Path::new("/tmp/a.db"));

        // Assert
        assert_eq!(f.draft.database, None);
    }
}
