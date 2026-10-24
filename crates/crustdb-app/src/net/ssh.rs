use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::BoxFuture;
use russh::client::{self, AuthResult, Handle, Handler};
use russh::keys::agent::{AgentIdentity, client::AgentClient};
use russh::keys::{HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKey, load_secret_key};
use secrecy::{ExposeSecret, SecretString};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};

use super::known_hosts::{HostKeyStatus, KnownHosts};
use crate::storage::config::{SshAuth, SshConfig};

const TIMEOUT: Duration = Duration::from_secs(10);

/// Receives the SHA256 fingerprint of an unknown host key; `true` accepts and saves it.
pub type HostKeyAsk = Box<dyn FnOnce(String) -> BoxFuture<'static, bool> + Send>;

#[derive(Default)]
pub struct SshSecrets {
    pub password: Option<SecretString>,
    pub passphrase: Option<SecretString>,
}

impl std::fmt::Debug for SshSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshSecrets")
            .field("password", &self.password.as_ref().map(|_| "***"))
            .field("passphrase", &self.passphrase.as_ref().map(|_| "***"))
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SshError {
    #[error("could not connect to the SSH server")]
    Connect,
    #[error("SSH authentication failed")]
    Auth,
    #[error("SSH host key rejected")]
    HostKeyRejected,
    #[error("SSH host key does not match the recorded one; possible man-in-the-middle")]
    HostKeyMismatch,
    #[error("SSH connection timed out")]
    Timeout,
    #[error("SSH io error: {0}")]
    Io(String),
}

/// Local forward on `127.0.0.1:<ephemeral>`; closed on drop.
pub struct Tunnel {
    pub local_addr: SocketAddr,
    task: JoinHandle<()>,
    handle: Arc<Handle<Client>>,
}

impl Tunnel {
    /// True once the SSH session is gone (connection lost or closed).
    pub fn is_closed(&self) -> bool {
        self.handle.is_closed()
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.task.abort(); // drops the JoinSet of forwarders and the SSH handle
    }
}

enum Seen {
    Unknown(Box<PublicKey>),
    Mismatch,
}

struct Client {
    known: KnownHosts,
    host: String,
    port: u16,
    seen: Arc<Mutex<Option<Seen>>>,
}

impl Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            return Ok(false);
        };
        let seen = match self.known.check(&self.host, self.port, key) {
            HostKeyStatus::Known => return Ok(true),
            HostKeyStatus::Unknown { .. } => Seen::Unknown(Box::new(key.clone())),
            HostKeyStatus::Mismatch => Seen::Mismatch,
        };
        if let Ok(mut g) = self.seen.lock() {
            *g = Some(seen);
        }
        Ok(false)
    }
}

/// Offer only the host key types already recorded for the host, so a known host is never
/// answered with an unrelated key type (which would look like a mismatch).
fn host_key_prefs(
    recorded: Vec<russh::keys::Algorithm>,
    supported: &[russh::keys::Algorithm],
) -> Result<Option<Vec<russh::keys::Algorithm>>, SshError> {
    if recorded.is_empty() {
        return Ok(None);
    }
    // A recorded RSA key (hash-less) is served under rsa-sha2-256/512; offer every RSA variant.
    let rsa = recorded.iter().any(|a| a.clone().is_rsa());
    let usable: Vec<_> = supported
        .iter()
        .filter(|a| recorded.contains(a) || (rsa && (**a).clone().is_rsa()))
        .cloned()
        .collect();
    if usable.is_empty() {
        return Err(SshError::Io(
            "host key type recorded in known_hosts is not supported by this client".into(),
        ));
    }
    Ok(Some(usable))
}

enum Fail {
    Err(SshError),
    Unknown(Box<PublicKey>),
}

async fn connect(
    cfg: &SshConfig,
    secrets: &SshSecrets,
    known: &KnownHosts,
) -> Result<Handle<Client>, Fail> {
    let seen = Arc::new(Mutex::new(None));
    let client = Client {
        known: known.clone(),
        host: cfg.host.clone(),
        port: cfg.port,
        seen: seen.clone(),
    };
    let mut config = client::Config::default();
    let recorded = known.algorithms(&cfg.host, cfg.port);
    match host_key_prefs(recorded, &config.preferred.key) {
        Ok(Some(prefs)) => config.preferred.key = std::borrow::Cow::Owned(prefs),
        Ok(None) => {}
        Err(e) => return Err(Fail::Err(e)),
    }
    let attempt = async {
        let sock = TcpStream::connect((cfg.host.as_str(), cfg.port))
            .await
            .map_err(|_| SshError::Connect)?;
        let mut h = client::connect_stream(Arc::new(config), sock, client)
            .await
            .map_err(|_| SshError::Connect)?;
        authenticate(&mut h, cfg, secrets).await?;
        Ok::<_, SshError>(h)
    };
    let res = tokio::time::timeout(TIMEOUT, attempt)
        .await
        .unwrap_or(Err(SshError::Timeout));
    let seen = seen.lock().ok().and_then(|mut g| g.take());
    match (res, seen) {
        (Ok(h), _) => Ok(h),
        (Err(_), Some(Seen::Unknown(k))) => Err(Fail::Unknown(k)),
        (Err(_), Some(Seen::Mismatch)) => Err(Fail::Err(SshError::HostKeyMismatch)),
        (Err(e), None) => Err(Fail::Err(e)),
    }
}

async fn authenticate(
    h: &mut Handle<Client>,
    cfg: &SshConfig,
    secrets: &SshSecrets,
) -> Result<(), SshError> {
    let ok = |r: AuthResult| {
        if r.success() {
            Ok(())
        } else {
            Err(SshError::Auth)
        }
    };
    match &cfg.auth {
        SshAuth::Password => {
            let pw = secrets.password.as_ref().ok_or(SshError::Auth)?;
            let r = h.authenticate_password(&cfg.user, pw.expose_secret()).await;
            ok(r.map_err(|_| SshError::Connect)?)
        }
        SshAuth::Key { key_path } => {
            let pass = secrets.passphrase.as_ref().map(|p| p.expose_secret());
            let key = load_key(key_path, pass)?;
            let hash = rsa_hash(h).await;
            let key = PrivateKeyWithHashAlg::new(Arc::new(key), hash);
            let r = h.authenticate_publickey(&cfg.user, key).await;
            ok(r.map_err(|_| SshError::Connect)?)
        }
        SshAuth::Agent => {
            let mut agent = AgentClient::connect_env()
                .await
                .map_err(|_| SshError::Auth)?;
            let ids = agent
                .request_identities()
                .await
                .map_err(|_| SshError::Auth)?;
            let hash = rsa_hash(h).await;
            for id in ids {
                let AgentIdentity::PublicKey { key, .. } = id else {
                    continue;
                };
                let r = h
                    .authenticate_publickey_with(&cfg.user, key, hash, &mut agent)
                    .await;
                if r.is_ok_and(|r| r.success()) {
                    return Ok(());
                }
            }
            Err(SshError::Auth)
        }
    }
}

fn load_key(path: &std::path::Path, pass: Option<&str>) -> Result<PrivateKey, SshError> {
    match load_secret_key(path, pass) {
        Ok(k) => Ok(k),
        Err(russh::keys::Error::UnsupportedKeyType { .. }) => {
            Err(SshError::Io("unsupported SSH key type".into()))
        }
        Err(_) => Err(SshError::Auth),
    }
}

async fn rsa_hash(h: &Handle<Client>) -> Option<HashAlg> {
    h.best_supported_rsa_hash().await.ok().flatten().flatten()
}

pub async fn open_tunnel(
    cfg: &SshConfig,
    secrets: SshSecrets,
    target: (String, u16),
    known: &KnownHosts,
    ask: HostKeyAsk,
) -> Result<Tunnel, SshError> {
    let handle = match connect(cfg, &secrets, known).await {
        Ok(h) => h,
        Err(Fail::Err(e)) => return Err(e),
        Err(Fail::Unknown(key)) => {
            let fp = key.fingerprint(HashAlg::Sha256).to_string();
            if !ask(fp).await {
                return Err(SshError::HostKeyRejected);
            }
            known
                .add(&cfg.host, cfg.port, &key)
                .map_err(|e| SshError::Io(e.to_string()))?;
            match connect(cfg, &secrets, known).await {
                Ok(h) => h,
                Err(Fail::Err(e)) => return Err(e),
                Err(Fail::Unknown(_)) => return Err(SshError::HostKeyMismatch),
            }
        }
    };
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| SshError::Io(e.to_string()))?;
    let local_addr = listener
        .local_addr()
        .map_err(|e| SshError::Io(e.to_string()))?;
    let handle = Arc::new(handle);
    let session = handle.clone();
    let task = tokio::spawn(async move {
        let mut conns = JoinSet::new();
        loop {
            let Ok((mut sock, peer)) = listener.accept().await else {
                // e.g. EMFILE: keep the tunnel alive and retry
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            };
            let (h, (host, port)) = (handle.clone(), target.clone());
            conns.spawn(async move {
                let Ok(ch) = h
                    .channel_open_direct_tcpip(host, port.into(), "127.0.0.1", peer.port().into())
                    .await
                else {
                    return;
                };
                let _ = tokio::io::copy_bidirectional(&mut sock, &mut ch.into_stream()).await;
            });
        }
    });
    Ok(Tunnel {
        local_addr,
        task,
        handle: session,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use russh::keys::Algorithm;

    #[test]
    fn host_key_prefs_keep_recorded_types_and_fail_when_none_usable() {
        // Arrange
        let rsa = Algorithm::Rsa { hash: None };
        let sup = client::Config::default().preferred.key;

        // Act
        let got = host_key_prefs(vec![rsa.clone(), Algorithm::Ed25519], &sup)
            .unwrap()
            .unwrap();

        // Assert
        assert!(got.contains(&Algorithm::Ed25519));
        assert!(
            got.iter().filter(|a| (*a).clone().is_rsa()).count() > 1,
            "{got:?}"
        );
        assert!(
            got.iter()
                .all(|a| a.clone().is_rsa() || *a == Algorithm::Ed25519)
        );
        assert!(matches!(
            host_key_prefs(vec![rsa], &[Algorithm::Ed25519]),
            Err(SshError::Io(_))
        ));
        assert_eq!(host_key_prefs(vec![], &sup).unwrap(), None);
    }

    fn keygen(dir: &std::path::Path, args: &[&str]) -> std::path::PathBuf {
        let f = dir.join("k");
        let ok = std::process::Command::new("ssh-keygen")
            .args(["-q", "-N", ""])
            .args(args)
            .arg("-f")
            .arg(&f)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        f
    }

    #[test]
    fn rsa_and_ed25519_keys_load_in_both_formats() {
        // Act & Assert
        for (ty, fmt) in [("rsa", "RFC4716"), ("rsa", "PEM"), ("ed25519", "RFC4716")] {
            let dir = tempfile::tempdir().unwrap();
            let f = keygen(dir.path(), &["-t", ty, "-m", fmt]);
            assert!(load_key(&f, None).is_ok(), "{ty} {fmt}");
        }
    }
}
