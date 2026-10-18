use crustdb_core::model::{ConnectionId, ServerVersion};

use super::dialogs::{Dialog, DialogStack};
use crate::runtime::{Event, WorkspaceId};
use crate::storage::config::{ConnectionConfig, LoadedConfig};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Connecting,
    Online(ServerVersion),
    Failed(String),
}

#[derive(Debug)]
pub struct WorkspaceState {
    pub id: WorkspaceId,
    pub config: ConnectionConfig,
    pub status: Status,
}

impl WorkspaceState {
    /// The connection form stays up until the workspace is online.
    pub fn form_open(&self) -> bool {
        !matches!(self.status, Status::Online(_))
    }
}

pub struct AppState {
    pub config: LoadedConfig,
    pub workspaces: Vec<WorkspaceState>,
    pub active: Option<WorkspaceId>,
    pub dialogs: DialogStack,
    next_ws: u64,
}

impl AppState {
    pub fn new(config: LoadedConfig) -> Self {
        Self {
            config,
            workspaces: vec![],
            active: None,
            dialogs: DialogStack::default(),
            // 0 is the runtime's sentinel.
            next_ws: 1,
        }
    }

    /// Id for a workspace that never appears in the UI (connection test).
    pub fn reserve_ws(&mut self) -> WorkspaceId {
        let id = WorkspaceId(self.next_ws);
        self.next_ws += 1;
        id
    }

    pub fn open_workspace(&mut self, config: ConnectionConfig) -> WorkspaceId {
        let id = WorkspaceId(self.next_ws);
        self.next_ws += 1;
        self.workspaces.push(WorkspaceState {
            id,
            config,
            status: Status::Connecting,
        });
        self.active = Some(id);
        id
    }

    pub fn workspace_mut(&mut self, id: WorkspaceId) -> Option<&mut WorkspaceState> {
        self.workspaces.iter_mut().find(|w| w.id == id)
    }

    pub fn apply(&mut self, e: Event) {
        match e {
            Event::Connected { ws, server } => {
                if let Some(w) = self.workspace_mut(ws) {
                    w.status = Status::Online(server);
                }
            }
            Event::ConnectFailed { ws, error } => {
                if let Some(w) = self.workspace_mut(ws) {
                    w.status = Status::Failed(error);
                }
            }
            _ => {}
        }
    }

    pub fn request_delete(&mut self, id: ConnectionId) {
        if self.config.read_only {
            return;
        }
        if let Some(c) = self.config.connections.iter().find(|c| c.id == id) {
            self.dialogs.push(Dialog::DeleteConnection {
                id,
                name: c.name.clone(),
            });
        }
    }

    /// Pops the confirmation dialog; the connection is removed only if `save` of the remaining
    /// list succeeds. Returns the deleted id.
    pub fn confirm_delete<E>(
        &mut self,
        save: impl FnOnce(&[ConnectionConfig]) -> Result<(), E>,
    ) -> Result<Option<ConnectionId>, E> {
        let Some(Dialog::DeleteConnection { id, .. }) = self.dialogs.pop() else {
            return Ok(None);
        };
        let rest: Vec<_> = self
            .config
            .connections
            .iter()
            .filter(|c| c.id != id)
            .cloned()
            .collect();
        save(&rest)?;
        self.config.connections = rest;
        Ok(Some(id))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crustdb_core::driver::{TlsConfig, TlsMode};
    use crustdb_core::model::{DriverKind, Product};

    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(1);

    pub(crate) fn conn(name: &str) -> ConnectionConfig {
        ConnectionConfig {
            id: ConnectionId(uuid::Uuid::from_u128(
                NEXT.fetch_add(1, Ordering::Relaxed).into(),
            )),
            name: name.into(),
            driver: DriverKind::Postgres,
            host: "localhost".into(),
            port: 5432,
            user: "u".into(),
            database: None,
            save_password: false,
            tls: TlsConfig {
                mode: TlsMode::VerifyFull,
                ca_path: None,
            },
            ssh: None,
            page_size: 300,
            editor_row_limit: 1000,
            safety_threshold: 1000,
        }
    }

    pub(crate) fn state(names: &[&str]) -> AppState {
        AppState::new(LoadedConfig {
            connections: names.iter().map(|n| conn(n)).collect(),
            read_only: false,
            warning: None,
        })
    }

    #[test]
    fn apply_connected_marks_workspace_online() {
        let mut s = state(&["a"]);
        let ws = s.open_workspace(s.config.connections[0].clone());
        assert_eq!(ws, WorkspaceId(1));
        let server = ServerVersion {
            product: Product::Postgres,
            major: 16,
            minor: 0,
            patch: 0,
            raw: "16.0".into(),
        };
        s.apply(Event::Connected {
            ws,
            server: server.clone(),
        });
        assert_eq!(s.workspaces[0].status, Status::Online(server));
        assert!(!s.workspaces[0].form_open());
    }

    #[test]
    fn apply_connect_failed_keeps_form_open_with_error() {
        let mut s = state(&["a"]);
        let ws = s.open_workspace(s.config.connections[0].clone());
        s.apply(Event::ConnectFailed {
            ws,
            error: "refused".into(),
        });
        assert_eq!(s.workspaces[0].status, Status::Failed("refused".into()));
        assert!(s.workspaces[0].form_open());
    }

    #[test]
    fn delete_connection_requires_confirmation() {
        let mut s = state(&["a", "b"]);
        let id = s.config.connections[0].id;
        s.request_delete(id);
        assert_eq!(s.config.connections.len(), 2);
        assert!(matches!(
            s.dialogs.top(),
            Some(Dialog::DeleteConnection { id: i, .. }) if *i == id
        ));
        assert_eq!(s.confirm_delete(|_| Ok::<_, ()>(())), Ok(Some(id)));
        assert_eq!(s.config.connections.len(), 1);
        assert!(s.dialogs.top().is_none());
    }

    #[test]
    fn read_only_config_refuses_delete() {
        let mut s = state(&["a"]);
        s.config.read_only = true;
        s.request_delete(s.config.connections[0].id);
        assert!(s.dialogs.top().is_none());
        assert_eq!(s.config.connections.len(), 1);
    }

    #[test]
    fn failed_save_keeps_connection() {
        let mut s = state(&["a"]);
        s.request_delete(s.config.connections[0].id);
        assert_eq!(s.confirm_delete(|_| Err("io")), Err("io"));
        assert_eq!(s.config.connections.len(), 1);
    }
}
