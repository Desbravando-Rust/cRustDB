//! Command line of a key-value workspace: tokenize, assess, (confirm), run, show the reply.
use std::time::Instant;

use crustdb_core::model::{ColumnMeta, Row};
use crustdb_core::safety::Verdict;
use crustdb_core::store::Reply;
use crustdb_redis::tokenize::tokenize;

use super::dialogs::{Dialog, PasswordInput, SessionAuth};
use super::{AppState, Status};
use crate::runtime::{Command, Event, RequestId, WorkspaceId};

#[derive(Debug, Default)]
enum Phase {
    #[default]
    Idle,
    /// Waiting for the verdict (or for the user to approve it); running again replaces it.
    Assessing(RequestId, Vec<Vec<u8>>),
    Running(RequestId),
}

/// What an assessment asks of the caller.
enum Assessed {
    Send(Box<Command>),
    Ask(Box<Dialog>),
    Nothing,
}

#[derive(Debug, Default)]
pub struct CommandPane {
    pub text: String,
    phase: Phase,
    pub error: Option<String>,
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Row>,
    pub selected: Option<usize>,
    pub anchor: Option<usize>,
}

impl CommandPane {
    pub fn is_running(&self) -> bool {
        matches!(self.phase, Phase::Running(_))
    }

    /// Splits the line into arguments (one `Vec<u8>` each, never re-joined) and asks the driver
    /// how dangerous it is. A syntax error is shown and nothing is sent.
    pub fn run(&mut self, ws: WorkspaceId, req: RequestId) -> Option<Command> {
        if self.is_running() {
            return None;
        }
        match tokenize(&self.text) {
            Ok(argv) => {
                self.error = None;
                self.phase = Phase::Assessing(req, argv.clone());
                Some(Command::AssessCommand { ws, req, argv })
            }
            Err(e) => {
                self.error = Some(e.to_string());
                None
            }
        }
    }

    fn awaits(&self, req: RequestId) -> bool {
        matches!(&self.phase, Phase::Assessing(r, _) if *r == req)
    }

    fn assessed(
        &mut self,
        ws: WorkspaceId,
        req: RequestId,
        result: &Result<(Verdict, String), String>,
    ) -> Assessed {
        let Phase::Assessing(r, argv) = &self.phase else {
            return Assessed::Nothing;
        };
        if *r != req {
            return Assessed::Nothing;
        }
        match result {
            Err(reason) => {
                self.phase = Phase::Idle;
                self.error = Some(format!("Refused: {reason}"));
                Assessed::Nothing
            }
            Ok((Verdict::Run, _)) => {
                let argv = argv.clone();
                self.phase = Phase::Running(req);
                Assessed::Send(Box::new(Command::RunCommand { ws, req, argv }))
            }
            Ok((verdict, reason)) => Assessed::Ask(Box::new(Dialog::StoreSafety {
                ws,
                req,
                argv: argv.clone(),
                verdict: *verdict,
                reason: reason.clone(),
                input: PasswordInput::default(),
                attempts: 0,
                shown_at: Instant::now(),
            })),
        }
    }

    /// The user approved request `req`; its reply is expected next.
    fn approved(&mut self, req: RequestId) {
        if self.awaits(req) {
            self.phase = Phase::Running(req);
        }
    }

    fn reply(&mut self, req: RequestId, result: &Result<Reply, String>) {
        if !matches!(self.phase, Phase::Running(r) if r == req) {
            return;
        }
        self.phase = Phase::Idle;
        match result {
            Ok(r) => {
                self.error = None;
                (self.columns, self.rows) = r.to_rows();
                self.selected = None;
                self.anchor = None;
            }
            Err(m) => self.error = Some(m.clone()),
        }
    }

    /// Asks the runtime to drop the editor session under the running command.
    pub fn cancel(&self, ws: WorkspaceId) -> Option<Command> {
        match self.phase {
            Phase::Running(req) => Some(Command::CancelStore { ws, req }),
            _ => None,
        }
    }

    pub(super) fn apply(&mut self, e: &Event) {
        if let Event::CommandReply { req, result } = e {
            self.reply(*req, result);
        }
    }
}

impl AppState {
    /// Entry point for `Intent::RunCommand` of a key-value workspace.
    pub fn run_command(&mut self, ws: WorkspaceId) -> Option<Command> {
        if self.dialogs.has_store_safety(ws) {
            return None;
        }
        let req = self.internal_req();
        let w = self.workspace_mut(ws)?;
        if !w.capabilities().command_editor {
            return None;
        }
        w.keyspace.as_mut()?.command.run(ws, req)
    }

    pub fn cancel_command(&self, ws: WorkspaceId) -> Option<Command> {
        let w = self.workspaces.iter().find(|w| w.id == ws)?;
        w.keyspace.as_ref()?.command.cancel(ws)
    }

    /// Routes a verdict: `Run` goes straight out, anything else waits in a dialog.
    pub(super) fn command_assessed(
        &mut self,
        req: RequestId,
        result: &Result<(Verdict, String), String>,
    ) {
        let Some(w) = self.workspaces.iter_mut().find(|w| {
            matches!(w.status, Status::Online(_))
                && w.keyspace.as_ref().is_some_and(|k| k.command.awaits(req))
        }) else {
            return;
        };
        let Some(ks) = w.keyspace.as_mut() else {
            return;
        };
        match ks.command.assessed(w.id, req, result) {
            Assessed::Send(cmd) => self.outbox.push(*cmd),
            Assessed::Ask(d) => self.dialogs.push(*d),
            Assessed::Nothing => {}
        }
    }

    /// Entry point for `Intent::ConfirmSafety`: approves the top dialog with the credentials of
    /// its own workspace.
    pub fn confirm_safety(&mut self, now: Instant) -> Option<Command> {
        let workspaces = &self.workspaces;
        let cmd = self.dialogs.submit_safety(
            |id| {
                let w = workspaces.iter().find(|w| w.id == id)?;
                Some(SessionAuth {
                    password: w.session_password.as_ref(),
                    database: w.config.database.as_deref().unwrap_or(""),
                })
            },
            now,
        );
        if let Some(Command::RunCommand { ws, req, .. }) = &cmd
            && let Some(ks) = self.workspace_mut(*ws).and_then(|w| w.keyspace.as_mut())
        {
            ks.command.approved(*req);
        }
        cmd
    }
}

#[cfg(test)]
#[path = "command_pane_tests.rs"]
mod tests;
