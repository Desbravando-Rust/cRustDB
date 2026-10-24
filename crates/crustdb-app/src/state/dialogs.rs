use std::fmt;
use std::time::{Duration, Instant};

use crustdb_core::model::ConnectionId;
use crustdb_core::safety::{Evaluation, Verdict};
use secrecy::{ExposeSecret, SecretString};
use zeroize::{Zeroize, Zeroizing};

use crate::runtime::{Command, RequestId, TabId, TxOp, WorkspaceId};

pub const MAX_PASSWORD_ATTEMPTS: u8 = 3;
/// Approvals this soon after the dialog appears are leftovers of a double click.
pub const APPROVAL_GRACE: Duration = Duration::from_millis(300);
const COMPARE_LEN: usize = 256;

/// Typed password; wiped on drop and never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct PasswordInput(pub Zeroizing<String>);

impl Default for PasswordInput {
    fn default() -> Self {
        // Preallocated so typing does not reallocate and leave copies behind.
        Self(Zeroizing::new(String::with_capacity(128)))
    }
}

impl PasswordInput {
    pub fn new(s: &str) -> Self {
        let mut p = Self::default();
        p.0.push_str(s);
        p
    }

    fn wipe(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for PasswordInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PasswordInput(***)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dialog {
    /// `req` is the evaluated batch; `exec` the id the approved run will use.
    Safety {
        ws: WorkspaceId,
        tab: TabId,
        req: RequestId,
        exec: RequestId,
        evaluation: Evaluation,
        input: PasswordInput,
        attempts: u8,
        shown_at: Instant,
    },
    CloseWithTx {
        tab: TabId,
    },
    DirtyFile {
        tab: TabId,
    },
    HostKey {
        ws: WorkspaceId,
        host: String,
        fingerprint: String,
    },
    DeleteConnection {
        id: ConnectionId,
        name: String,
    },
}

/// What the session was opened with, for the password prompt (D25).
#[derive(Clone, Copy)]
pub struct SessionAuth<'a> {
    pub password: Option<&'a SecretString>,
    pub database: &'a str,
}

/// Compares against the session password; if it was empty or absent, against the database name
/// (D25). An empty secret never matches. Constant time: always walks a fixed length and folds the
/// length difference into the accumulator, with no early return.
pub fn verify_password(input: &str, expected: Option<&SecretString>, database: &str) -> bool {
    let expected = expected
        .map(|s| s.expose_secret())
        .filter(|s| !s.is_empty())
        .unwrap_or(database);
    let (a, b) = (input.as_bytes(), expected.as_bytes());
    let mut acc = (a.len() ^ b.len()) as u64;
    for i in 0..COMPARE_LEN.max(a.len()).max(b.len()) {
        acc |= u64::from(a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0));
    }
    acc == 0 && !b.is_empty()
}

#[derive(Debug, Default)]
pub struct DialogStack(Vec<Dialog>);

impl DialogStack {
    pub fn push(&mut self, d: Dialog) {
        self.0.push(d);
    }

    pub fn pop(&mut self) -> Option<Dialog> {
        self.0.pop()
    }

    /// Drops the dialogs that belong to a workspace or to one of its tabs.
    pub fn retain_ws(&mut self, ws: WorkspaceId, tabs: &[TabId]) {
        let _ = (ws, tabs);
    }

    pub fn top(&self) -> Option<&Dialog> {
        self.0.last()
    }

    pub fn top_mut(&mut self) -> Option<&mut Dialog> {
        self.0.last_mut()
    }

    /// `Run` executes straight away; anything else waits for approval in a dialog.
    pub fn on_evaluated(
        &mut self,
        ws: WorkspaceId,
        tab: TabId,
        req: RequestId,
        exec: RequestId,
        evaluation: Evaluation,
        now: Instant,
    ) -> Option<Command> {
        if evaluation.verdict == Verdict::Run {
            return Some(Command::Execute {
                tab,
                req: exec,
                evaluation: req,
            });
        }
        self.push(Dialog::Safety {
            ws,
            tab,
            req,
            exec,
            evaluation,
            input: PasswordInput::default(),
            attempts: 0,
            shown_at: now,
        });
        None
    }

    /// Approves the top safety dialog: a plain confirmation, or the typed password of ITS
    /// workspace (3 wrong attempts close it without executing). `auth` yields that workspace's
    /// session credentials; if it is gone, the dialog is dropped without executing.
    pub fn submit_safety<'a>(
        &mut self,
        auth: impl FnOnce(WorkspaceId) -> Option<SessionAuth<'a>>,
        now: Instant,
    ) -> Option<Command> {
        let Some(Dialog::Safety {
            ws,
            tab,
            req,
            exec,
            evaluation,
            input,
            attempts,
            shown_at,
        }) = self.0.last_mut()
        else {
            return None;
        };
        if now.saturating_duration_since(*shown_at) < APPROVAL_GRACE {
            return None;
        }
        let Some(a) = auth(*ws) else {
            self.0.pop();
            return None;
        };
        let needs_password = evaluation.verdict == Verdict::Password
            || evaluation
                .flags
                .iter()
                .any(|f| f.verdict == Verdict::Password);
        let approved = !needs_password || verify_password(&input.0, a.password, a.database);
        if approved {
            let cmd = Command::Execute {
                tab: *tab,
                req: *exec,
                evaluation: *req,
            };
            self.0.pop();
            return Some(cmd);
        }
        input.wipe();
        *attempts += 1;
        if *attempts >= MAX_PASSWORD_ATTEMPTS {
            self.0.pop();
        }
        None
    }

    /// Dismisses the top dialog. Only a host key prompt answers the runtime (a rejection), since
    /// the connection is waiting on it.
    pub fn cancel(&mut self) -> Option<Command> {
        match self.0.pop()? {
            Dialog::HostKey { ws, .. } => Some(Command::HostKeyDecision { ws, accept: false }),
            _ => None,
        }
    }

    pub fn accept_host_key(&mut self) -> Option<Command> {
        let Some(Dialog::HostKey { ws, .. }) = self.0.last() else {
            return None;
        };
        let cmd = Command::HostKeyDecision {
            ws: *ws,
            accept: true,
        };
        self.0.pop();
        Some(cmd)
    }

    /// Closes the tab after resolving its open transaction; `Commit` or `Rollback` only.
    pub fn close_with_tx(&mut self, op: TxOp) -> Option<Command> {
        let Some(Dialog::CloseWithTx { tab }) = self.0.last() else {
            return None;
        };
        if !matches!(op, TxOp::Commit | TxOp::Rollback) {
            return None;
        }
        let cmd = Command::CloseTab {
            tab: *tab,
            resolve: Some(op),
        };
        self.0.pop();
        Some(cmd)
    }

    pub fn discard_dirty(&mut self) -> Option<Command> {
        let Some(Dialog::DirtyFile { tab }) = self.0.last() else {
            return None;
        };
        let cmd = Command::CloseTab {
            tab: *tab,
            resolve: None,
        };
        self.0.pop();
        Some(cmd)
    }
}

#[cfg(test)]
#[path = "dialogs_tests.rs"]
mod tests;
