//! Following a foreign key from a Data grid cell to the referenced rows.
use crustdb_core::model::{Cell, ObjectRef};
use crustdb_core::query::{Filter, Operator};

use super::filter_draft::FilterDraft;
use super::workspace::{AppState, Tab};
use crate::runtime::{Command, RequestId, TabId, WorkspaceId};

impl AppState {
    /// Opens (or focuses) the table referenced by the FK that covers cell `(row, col)`, filtered to
    /// the referenced row. Does nothing for non-FK columns or NULL, truncated and binary values.
    pub fn follow_fk(
        &mut self,
        tab: TabId,
        row: usize,
        col: usize,
        req: RequestId,
    ) -> Vec<Command> {
        let Some((ws, object, drafts)) = self.fk_target(tab, row, col) else {
            return vec![];
        };
        let mut cmds = self.open_table(ws, object.clone(), req);
        let created = !cmds.is_empty();
        let Some(t) = self.workspace_mut(ws).and_then(|w| {
            w.tabs
                .iter_mut()
                .map(|Tab::Table(t)| t)
                .find(|t| t.object == object)
        }) else {
            return cmds;
        };
        t.filter_open = true;
        if created {
            // The first page is fetched once the structure arrives, already filtered.
            t.filters = drafts
                .iter()
                .map(|d| Filter {
                    column: d.column.clone(),
                    op: d.op,
                    values: vec![d.input.clone()],
                })
                .collect();
            t.drafts = drafts;
        } else {
            t.drafts = drafts;
            cmds = t.apply_filters(req).into_iter().collect();
        }
        cmds
    }

    fn fk_target(
        &self,
        tab: TabId,
        row: usize,
        col: usize,
    ) -> Option<(WorkspaceId, ObjectRef, Vec<FilterDraft>)> {
        let (ws, t) = self.workspaces.iter().find_map(|w| {
            let t = w.tabs.iter().map(|Tab::Table(t)| t).find(|t| t.id == tab)?;
            Some((w.id, t))
        })?;
        let name = &t.columns.get(col)?.name;
        let fk = t
            .structure
            .as_ref()?
            .fks
            .iter()
            .find(|f| f.columns.contains(name))?;
        let cells = t.rows.get(row)?;
        let drafts = fk
            .columns
            .iter()
            .zip(&fk.ref_columns)
            .map(|(c, ref_col)| {
                let j = t.columns.iter().position(|m| &m.name == c)?;
                match cells.get(j)? {
                    Cell::Text {
                        value,
                        truncated: false,
                    } => Some(FilterDraft {
                        column: ref_col.clone(),
                        op: Operator::Eq,
                        input: value.to_string(),
                    }),
                    _ => None,
                }
            })
            .collect::<Option<Vec<_>>>()?;
        Some((ws, fk.ref_table.clone(), drafts))
    }
}

#[cfg(test)]
#[path = "workspace_fk_tests.rs"]
mod tests;
