//! Connection setup shared by every workspace actor: optional SSH tunnel plus `ConnectTarget`.
use std::path::PathBuf;
use std::time::Duration;

use crustdb_core::driver::ConnectTarget;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use super::workspace::WsMsg;
use crate::net::known_hosts::KnownHosts;
use crate::net::ssh::{HostKeyAsk, Tunnel, open_tunnel};
use crate::runtime::{Emitter, Event, ResolvedSecrets, WorkspaceId};
use crate::storage::config::ConnectionConfig;
use crate::storage::paths::config_dir;

pub(super) const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Opens the SSH tunnel when configured (asking the user about unknown host keys through
/// `WsMsg::AskHostKey` + `Event::HostKeyPrompt`) and builds the target to connect to.
pub(super) async fn tunnel_target(
    config: &ConnectionConfig,
    secrets: ResolvedSecrets,
    ws: WorkspaceId,
    out: &Emitter,
    tx: &UnboundedSender<WsMsg>,
) -> Result<(Option<Tunnel>, ConnectTarget), String> {
    let mut tunnel = None;
    if let Some(ssh) = &config.ssh {
        let known = KnownHosts {
            app_file: config_dir().join("known_hosts"),
            user_file: std::env::var_os("HOME")
                .map_or_else(PathBuf::new, PathBuf::from)
                .join(".ssh/known_hosts"),
        };
        let (host, out, tx) = (ssh.host.clone(), out.clone(), tx.clone());
        let ask: HostKeyAsk = Box::new(move |fingerprint| {
            Box::pin(async move {
                let (d_tx, d_rx) = oneshot::channel();
                // Registered before the prompt, so the decision cannot overtake it.
                if tx.send(WsMsg::AskHostKey(d_tx)).is_err() {
                    return false;
                }
                out.emit(Event::HostKeyPrompt {
                    ws,
                    host,
                    fingerprint,
                });
                d_rx.await.unwrap_or(false)
            })
        });
        tunnel = Some(
            open_tunnel(
                ssh,
                secrets.ssh,
                (config.host.clone(), config.port),
                &known,
                ask,
            )
            .await
            .map_err(|e| e.to_string())?,
        );
    }
    let target = ConnectTarget {
        host: config.host.clone(),
        port: config.port,
        connect_addr: tunnel.as_ref().map(|t| t.local_addr),
        user: config.user.clone(),
        password: secrets.db,
        database: config.database.clone(),
        tls: config.tls.clone(),
        connect_timeout: CONNECT_TIMEOUT,
    };
    Ok((tunnel, target))
}
