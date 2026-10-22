#![allow(clippy::unwrap_used, clippy::expect_used, clippy::err_expect)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crustdb_app::net::known_hosts::KnownHosts;
use crustdb_app::net::ssh::{HostKeyAsk, SshError, SshSecrets, Tunnel, open_tunnel};
use crustdb_app::storage::config::{SshAuth, SshConfig};
use crustdb_core::driver::{ConnectTarget, Driver, SessionPurpose, TlsConfig, TlsMode};
use crustdb_core::error::ErrorKind;
use crustdb_postgres::PostgresDriver;
use crustdb_testkit::{SshContainer, certs_for, start_postgres, start_sshd};
use russh::keys::{HashAlg, PublicKey};
use secrecy::SecretString;

const DB_NAME: &str = "db.internal.test";
const PASSPHRASE: &str = "correct horse";

/// One docker network per test: tests run in parallel.
fn network(test: &str) -> String {
    format!("crustdb-ssh-{}-{test}", std::process::id())
}

fn cfg(ssh: &SshContainer, auth: SshAuth) -> SshConfig {
    SshConfig {
        host: ssh.host.clone(),
        port: ssh.port,
        user: ssh.user.clone(),
        auth,
    }
}

fn key_auth(ssh: &SshContainer) -> SshAuth {
    SshAuth::Key {
        key_path: ssh.client_key_path.clone(),
    }
}

fn known(dir: &Path) -> KnownHosts {
    KnownHosts {
        app_file: dir.join("app/known_hosts"),
        user_file: dir.join("none"),
    }
}

fn host_key(ssh: &SshContainer) -> PublicKey {
    ssh.host_public_key.parse().unwrap()
}

fn answer(accept: bool) -> HostKeyAsk {
    Box::new(move |_| Box::pin(async move { accept }))
}

async fn open(
    ssh: &SshContainer,
    auth: SshAuth,
    secrets: SshSecrets,
    target: &str,
    k: &KnownHosts,
    ask: HostKeyAsk,
) -> Result<Tunnel, SshError> {
    open_tunnel(&cfg(ssh, auth), secrets, (target.into(), 5432), k, ask).await
}

fn pg_target(pg_password: &str, host: &str, t: &Tunnel, ca: PathBuf) -> ConnectTarget {
    ConnectTarget {
        host: host.into(),
        port: 5432,
        connect_addr: Some(t.local_addr),
        user: "postgres".into(),
        password: Some(SecretString::from(pg_password.to_string())),
        database: Some("postgres".into()),
        tls: TlsConfig {
            mode: TlsMode::VerifyFull,
            ca_path: Some(ca),
        },
        connect_timeout: Duration::from_secs(10),
    }
}

fn keygen_passphrase_copy(src: &Path, dir: &Path) -> PathBuf {
    let dst = dir.join("id_pass");
    std::fs::copy(src, &dst).unwrap();
    let ok = Command::new("ssh-keygen")
        .args(["-q", "-p", "-P", "", "-N", PASSPHRASE, "-f"])
        .arg(&dst)
        .status()
        .unwrap()
        .success();
    assert!(ok, "ssh-keygen -p failed");
    dst
}

struct Agent(std::process::Child);
impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

fn start_agent(sock: &Path, key: &Path) -> Agent {
    let child = Command::new("ssh-agent")
        .args(["-D", "-a"])
        .arg(sock)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let agent = Agent(child);
    for _ in 0..50 {
        if sock.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let ok = Command::new("ssh-add")
        .arg(key)
        .env("SSH_AUTH_SOCK", sock)
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success();
    assert!(ok, "ssh-add failed");
    agent
}

#[tokio::test]
#[ignore = "docker"]
async fn ssh_tunnel_with_key_password_agent() {
    let net = network("modes");
    let ssh = start_sshd(&net).await;
    let certs = certs_for(&[DB_NAME]);
    let pg = start_postgres(Some(&certs), Some(&net)).await;
    let dir = tempfile::tempdir().unwrap();
    let k = known(dir.path());
    k.add(&ssh.host, ssh.port, &host_key(&ssh)).unwrap();
    let ca = certs.dir.path().join("ca.pem");
    let pass_key = keygen_passphrase_copy(&ssh.client_key_path, dir.path());

    let cases = [
        (
            SshAuth::Password,
            SshSecrets {
                password: Some(SecretString::from(ssh.password.clone())),
                passphrase: None,
            },
        ),
        (key_auth(&ssh), SshSecrets::default()),
        (
            SshAuth::Key { key_path: pass_key },
            SshSecrets {
                password: None,
                passphrase: Some(SecretString::from(PASSPHRASE)),
            },
        ),
    ];
    for (auth, secrets) in cases {
        let label = format!("{auth:?}");
        let t = open(&ssh, auth, secrets, &pg.alias, &k, answer(false))
            .await
            .unwrap_or_else(|e| panic!("{label}: {e}"));
        let target = pg_target(&pg.password, DB_NAME, &t, ca.clone());
        let mut s = PostgresDriver
            .connect(&target, SessionPurpose::Editor)
            .await
            .unwrap_or_else(|e| panic!("{label}: {e}"));
        s.ping().await.unwrap();
        s.close().await;
    }

    // ssh-agent: SSH_AUTH_SOCK cannot be set in-process (no unsafe), so a child test process runs it.
    let sock = dir.path().join("agent.sock");
    let _agent = start_agent(&sock, &ssh.client_key_path);
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["agent_child", "--exact", "--ignored", "--nocapture"])
        .env("SSH_AUTH_SOCK", &sock)
        .env("CRUSTDB_T_HOST", &ssh.host)
        .env("CRUSTDB_T_PORT", ssh.port.to_string())
        .env("CRUSTDB_T_USER", &ssh.user)
        .env("CRUSTDB_T_TARGET", &pg.alias)
        .env("CRUSTDB_T_KNOWN", &k.app_file)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "agent child failed: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[tokio::test]
#[ignore = "docker"]
async fn ssh_tunnel_with_rsa_user_key_and_rsa_host_key() {
    let net = network("rsa");
    let ssh = start_sshd(&net).await;
    let dir = tempfile::tempdir().unwrap();
    let k = known(dir.path());
    // known_hosts holds only the RSA host key, so the client must offer and verify RSA.
    k.add(
        &ssh.host,
        ssh.port,
        &ssh.host_rsa_public_key.parse().unwrap(),
    )
    .unwrap();
    let auth = SshAuth::Key {
        key_path: ssh.rsa_client_key_path.clone(),
    };
    open(
        &ssh,
        auth,
        SshSecrets::default(),
        "127.0.0.1",
        &k,
        answer(false),
    )
    .await
    .unwrap_or_else(|e| panic!("rsa: {e}"));
}

/// Runs inside the child process of `ssh_tunnel_with_key_password_agent`; no-op otherwise.
#[tokio::test]
#[ignore = "child of ssh_tunnel_with_key_password_agent"]
async fn agent_child() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let Ok(host) = std::env::var("CRUSTDB_T_HOST") else {
        return;
    };
    let var = |n: &str| std::env::var(n).unwrap();
    let config = SshConfig {
        host,
        port: var("CRUSTDB_T_PORT").parse().unwrap(),
        user: var("CRUSTDB_T_USER"),
        auth: SshAuth::Agent,
    };
    let k = KnownHosts {
        app_file: var("CRUSTDB_T_KNOWN").into(),
        user_file: "/nonexistent".into(),
    };
    let target = (var("CRUSTDB_T_TARGET"), 5432);
    let t = open_tunnel(&config, SshSecrets::default(), target, &k, answer(false))
        .await
        .unwrap();
    // PostgreSQL SSLRequest: the server answers 'S' or 'N' only through a working tunnel.
    let mut sock = tokio::net::TcpStream::connect(t.local_addr).await.unwrap();
    sock.write_all(&[0, 0, 0, 8, 4, 0xd2, 0x16, 0x2f])
        .await
        .unwrap();
    let mut b = [0u8; 1];
    sock.read_exact(&mut b).await.unwrap();
    assert!(matches!(b[0], b'S' | b'N'));
}

#[tokio::test]
#[ignore = "docker"]
async fn unknown_host_key_rejected() {
    let ssh = start_sshd(&network("reject")).await;
    let dir = tempfile::tempdir().unwrap();
    let k = known(dir.path());
    let err = open(
        &ssh,
        key_auth(&ssh),
        SshSecrets::default(),
        "x",
        &k,
        answer(false),
    )
    .await
    .err()
    .expect("must fail");
    assert_eq!(err, SshError::HostKeyRejected);
    assert!(!k.app_file.exists());
}

#[tokio::test]
#[ignore = "docker"]
async fn accepted_key_saved_to_app_file() {
    let ssh = start_sshd(&network("accept")).await;
    let dir = tempfile::tempdir().unwrap();
    let k = known(dir.path());
    let (tx, rx) = tokio::sync::oneshot::channel();
    let ask: HostKeyAsk = Box::new(move |fp| {
        let _ = tx.send(fp);
        Box::pin(async { true })
    });
    let _t = open(&ssh, key_auth(&ssh), SshSecrets::default(), "x", &k, ask)
        .await
        .unwrap();
    let key = host_key(&ssh);
    assert_eq!(
        rx.await.unwrap(),
        key.fingerprint(HashAlg::Sha256).to_string()
    );
    let saved = std::fs::read_to_string(&k.app_file).unwrap();
    assert!(saved.contains(key.to_openssh().unwrap().split(' ').nth(1).unwrap()));
    // Now known: a second open must not need the ask (it would answer "reject").
    open(
        &ssh,
        key_auth(&ssh),
        SshSecrets::default(),
        "x",
        &k,
        answer(false),
    )
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "docker"]
async fn changed_host_key_blocks() {
    let ssh = start_sshd(&network("changed")).await;
    let dir = tempfile::tempdir().unwrap();
    let k = known(dir.path());
    let other = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAILlalEU61txklSBkku8n1ZxRgOGLYm8YtbqsDdo1Ga4j";
    k.add(&ssh.host, ssh.port, &other.parse().unwrap()).unwrap();
    let err = open(
        &ssh,
        key_auth(&ssh),
        SshSecrets::default(),
        "x",
        &k,
        answer(true),
    )
    .await
    .err()
    .expect("must fail");
    assert_eq!(err, SshError::HostKeyMismatch);
}

#[tokio::test]
#[ignore = "docker"]
async fn verify_full_via_ssh_checks_real_hostname() {
    let net = network("hostname");
    let ssh = start_sshd(&net).await;
    let certs = certs_for(&[DB_NAME]);
    let pg = start_postgres(Some(&certs), Some(&net)).await;
    let dir = tempfile::tempdir().unwrap();
    let k = known(dir.path());
    k.add(&ssh.host, ssh.port, &host_key(&ssh)).unwrap();
    let t = open(
        &ssh,
        key_auth(&ssh),
        SshSecrets::default(),
        &pg.alias,
        &k,
        answer(false),
    )
    .await
    .unwrap();
    // The tunnel endpoint is 127.0.0.1, which the certificate does not cover: only the real name passes.
    let ca = certs.dir.path().join("ca.pem");
    let wrong = pg_target(&pg.password, "127.0.0.1", &t, ca.clone());
    let err = PostgresDriver
        .connect(&wrong, SessionPurpose::Editor)
        .await
        .err()
        .expect("must fail");
    assert_eq!(err.kind, ErrorKind::Tls, "{err}");
    let right = pg_target(&pg.password, DB_NAME, &t, ca);
    let s = PostgresDriver
        .connect(&right, SessionPurpose::Editor)
        .await
        .unwrap();
    s.close().await;
}
