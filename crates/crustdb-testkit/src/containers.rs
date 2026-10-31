use crate::TestCerts;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU32, Ordering},
    time::{Duration, Instant},
};
use testcontainers::{
    ContainerAsync, CopyTargetOptions, GenericImage, ImageExt,
    core::{ExecCommand, IntoContainerPort},
    runners::AsyncRunner,
};

const READY_TIMEOUT: Duration = Duration::from_secs(180);
const PASSWORD: &str = "crustdb";

// Fields: connect from the host with `host`/`port`; from another container on the
// same network use `alias` and the server's default port.
pub struct PgContainer {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub alias: String,
    guard: ContainerAsync<GenericImage>,
}

impl PgContainer {
    /// Stops and starts the container (connections drop; the host port may change).
    pub async fn restart(&self) {
        self.guard.stop_with_timeout(Some(1)).await.unwrap();
        self.guard.start().await.unwrap();
    }
}

pub struct MysqlContainer {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub alias: String,
    guard: ContainerAsync<GenericImage>,
}

impl MysqlContainer {
    /// Stops and starts the container (connections drop; the host port may change).
    pub async fn restart(&self) {
        self.guard.stop_with_timeout(Some(1)).await.unwrap();
        self.guard.start().await.unwrap();
    }
}

/// Redis protected by `requirepass`; with TLS, only the TLS port (6379) is served.
pub struct RedisContainer {
    pub host: String,
    pub port: u16,
    pub password: String,
    _guard: ContainerAsync<GenericImage>,
}

pub struct SshContainer {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub alias: String,
    /// Server host public key, OpenSSH one-line format.
    pub host_public_key: String,
    /// Unencrypted OpenSSH private key authorized for `user`.
    pub client_key_path: PathBuf,
    /// Server RSA host public key, OpenSSH one-line format.
    pub host_rsa_public_key: String,
    /// Unencrypted RSA private key authorized for `user`.
    pub rsa_client_key_path: PathBuf,
    _dir: tempfile::TempDir,
    _guard: ContainerAsync<GenericImage>,
}

fn image(var: &str, default: &str) -> GenericImage {
    let full = std::env::var(var).unwrap_or_else(|_| default.to_string());
    let (name, tag) = match full.rsplit_once(':') {
        Some((n, t)) if !t.contains('/') => (n, t),
        _ => (full.as_str(), "latest"),
    };
    GenericImage::new(name, tag)
}

fn unique(prefix: &str) -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    format!(
        "{prefix}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

fn copy_tls(tls: &TestCerts, dir: &str) -> Vec<(CopyTargetOptions, PathBuf)> {
    ["ca.pem", "server.crt", "server.key"]
        .map(|f| {
            (
                CopyTargetOptions::new(format!("{dir}/{f}")).with_mode(0o644),
                tls.dir.path().join(f),
            )
        })
        .into()
}

/// Runs `cmd` in the container until it exits 0 or the timeout elapses.
async fn wait_exec(c: &ContainerAsync<GenericImage>, cmd: &[&str]) {
    let start = Instant::now();
    loop {
        if let Ok(mut r) = c.exec(ExecCommand::new(cmd.iter().copied())).await
            && r.stdout_to_vec().await.is_ok()
            && r.exit_code().await.ok().flatten() == Some(0)
        {
            return;
        }
        assert!(
            start.elapsed() < READY_TIMEOUT,
            "container not ready: {cmd:?}"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn exec_out(c: &ContainerAsync<GenericImage>, script: &str) -> String {
    let mut r = c
        .exec(ExecCommand::new(["sh", "-c", script]))
        .await
        .unwrap();
    String::from_utf8(r.stdout_to_vec().await.unwrap()).unwrap()
}

async fn endpoint(c: &ContainerAsync<GenericImage>, internal: u16) -> (String, u16) {
    (
        c.get_host().await.unwrap().to_string(),
        c.get_host_port_ipv4(internal.tcp()).await.unwrap(),
    )
}

/// Image from `CRUSTDB_PG_IMAGE` (default `postgres:latest`). With `tls`, SSL is on
/// and the cert is valid for the hostnames given to `certs_for`.
pub async fn start_postgres(tls: Option<&TestCerts>, network: Option<&str>) -> PgContainer {
    start_pg(tls, network, PgAccess::Any).await
}

/// Like `start_postgres` with TLS, but a `pg_hba.conf` that rejects non-TLS TCP connections.
pub async fn start_postgres_tls_only(tls: &TestCerts) -> PgContainer {
    start_pg(Some(tls), None, PgAccess::TlsOnly).await
}

/// Like `start_postgres_tls_only`, and the client must present a certificate signed by the CA.
pub async fn start_postgres_mtls(tls: &TestCerts) -> PgContainer {
    start_pg(Some(tls), None, PgAccess::ClientCert).await
}

#[derive(Clone, Copy, PartialEq)]
enum PgAccess {
    Any,
    TlsOnly,
    ClientCert,
}

async fn start_pg(tls: Option<&TestCerts>, network: Option<&str>, access: PgAccess) -> PgContainer {
    let alias = unique("crustdb-pg");
    let mut img = image("CRUSTDB_PG_IMAGE", "postgres:latest");
    if tls.is_some() {
        img = img.with_entrypoint("sh");
    }
    let mut req = img
        .with_exposed_port(5432.tcp())
        .with_env_var("POSTGRES_PASSWORD", PASSWORD)
        .with_container_name(&alias);
    if let Some(t) = tls {
        // The key must belong to the server user with mode 0600: fix that, then run the stock entrypoint.
        for (target, src) in copy_tls(t, "/tmp/tls") {
            req = req.with_copy_to(target, src);
        }
        let hba = match access {
            PgAccess::Any => "",
            PgAccess::TlsOnly => {
                "printf 'local all all trust\\nhostnossl all all all reject\\nhostssl all all all scram-sha-256\\n' > /tmp/tls/hba.conf \
             && "
            }
            PgAccess::ClientCert => {
                "printf 'local all all trust\\nhostnossl all all all reject\\nhostssl all all all scram-sha-256 clientcert=verify-ca\\n' > /tmp/tls/hba.conf \
             && "
            }
        };
        let hba_opt = if access != PgAccess::Any {
            "-c hba_file=/tmp/tls/hba.conf"
        } else {
            ""
        };
        req = req.with_cmd([
            "-c".to_string(),
            format!("{hba}chown postgres /tmp/tls/* && chmod 600 /tmp/tls/server.key && exec docker-entrypoint.sh postgres {hba_opt} \
             -c ssl=on -c ssl_cert_file=/tmp/tls/server.crt -c ssl_key_file=/tmp/tls/server.key \
             -c ssl_ca_file=/tmp/tls/ca.pem"),
        ]);
    }
    if let Some(n) = network {
        req = req.with_network(n);
    }
    let c = req.start().await.unwrap();
    // TCP only: the init-time temporary server listens on the unix socket alone.
    wait_exec(&c, &["pg_isready", "-h", "127.0.0.1", "-U", "postgres"]).await;
    let (host, port) = endpoint(&c, 5432).await;
    PgContainer {
        host,
        port,
        user: "postgres".into(),
        password: PASSWORD.into(),
        alias,
        guard: c,
    }
}

/// Image from `CRUSTDB_MYSQL_IMAGE` (default `mysql:8.4`; MariaDB works too).
pub async fn start_mysql(tls: Option<&TestCerts>, network: Option<&str>) -> MysqlContainer {
    start_my(tls, network, false).await
}

/// TLS server where `root` over TCP must present a CA-signed client certificate (`REQUIRE X509`).
pub async fn start_mysql_mtls(tls: &TestCerts) -> MysqlContainer {
    start_my(Some(tls), None, true).await
}

async fn start_my(tls: Option<&TestCerts>, network: Option<&str>, x509: bool) -> MysqlContainer {
    let alias = unique("crustdb-mysql");
    let mut req = image("CRUSTDB_MYSQL_IMAGE", "mysql:8.4")
        .with_exposed_port(3306.tcp())
        .with_env_var("MYSQL_ROOT_PASSWORD", PASSWORD)
        .with_container_name(&alias);
    if let Some(t) = tls {
        for (target, src) in copy_tls(t, "/etc/crustdb-tls") {
            req = req.with_copy_to(target, src);
        }
        req = req.with_cmd([
            "--ssl-ca=/etc/crustdb-tls/ca.pem",
            "--ssl-cert=/etc/crustdb-tls/server.crt",
            "--ssl-key=/etc/crustdb-tls/server.key",
        ]);
    }
    if let Some(n) = network {
        req = req.with_network(n);
    }
    let c = req.start().await.unwrap();
    // TCP ping: the init-time temporary server is skip-networking. MariaDB 11+ may lack the mysqladmin name.
    wait_exec(&c, &["sh", "-c", "mysqladmin --protocol=tcp -h127.0.0.1 ping --silent || mariadb-admin --protocol=tcp -h127.0.0.1 ping --silent"]).await;
    if x509 {
        // Over the local socket: root@localhost is not affected, only root@'%' (TCP).
        wait_exec(&c, &["sh", "-c", "(mysql -uroot -pcrustdb || mariadb -uroot -pcrustdb) -e \"ALTER USER 'root'@'%' REQUIRE X509\""]).await;
    }
    let (host, port) = endpoint(&c, 3306).await;
    MysqlContainer {
        host,
        port,
        user: "root".into(),
        password: PASSWORD.into(),
        alias,
        guard: c,
    }
}

/// Image from `CRUSTDB_REDIS_IMAGE` (default `redis:latest`). With `tls`, plain TCP is off and
/// the cert is valid for the hostnames given to `certs_for`.
pub async fn start_redis(tls: Option<&TestCerts>) -> RedisContainer {
    let mut args = vec!["--requirepass", PASSWORD];
    let mut cli = vec!["redis-cli", "--no-auth-warning", "-a", PASSWORD];
    if tls.is_some() {
        args.extend([
            "--port",
            "0",
            "--tls-port",
            "6379",
            "--tls-cert-file",
            "/etc/crustdb-tls/server.crt",
            "--tls-key-file",
            "/etc/crustdb-tls/server.key",
            "--tls-ca-cert-file",
            "/etc/crustdb-tls/ca.pem",
            "--tls-auth-clients",
            "no",
        ]);
        cli.extend(["--tls", "--insecure"]);
    }
    let mut req = image("CRUSTDB_REDIS_IMAGE", "redis:latest")
        .with_exposed_port(6379.tcp())
        .with_cmd(args);
    if let Some(t) = tls {
        for (target, src) in copy_tls(t, "/etc/crustdb-tls") {
            req = req.with_copy_to(target, src);
        }
    }
    let c = req.start().await.unwrap();
    cli.push("ping");
    wait_exec(&c, &cli).await;
    let (host, port) = endpoint(&c, 6379).await;
    RedisContainer {
        host,
        port,
        password: PASSWORD.into(),
        _guard: c,
    }
}

/// `linuxserver/openssh-server`; creates `network` if missing. Password and key auth both work.
pub async fn start_sshd(network: &str) -> SshContainer {
    let alias = unique("crustdb-sshd");
    let c = GenericImage::new("linuxserver/openssh-server", "latest")
        .with_exposed_port(2222.tcp())
        .with_env_var("PASSWORD_ACCESS", "true")
        .with_env_var("USER_NAME", "crustdb")
        .with_env_var("USER_PASSWORD", PASSWORD)
        .with_container_name(&alias)
        .with_network(network)
        .start()
        .await
        .unwrap();
    // Host keys exist once the init scripts are done.
    wait_exec(
        &c,
        &[
            "sh",
            "-c",
            "test -s /config/ssh_host_keys/ssh_host_ed25519_key.pub && nc -z 127.0.0.1 2222",
        ],
    )
    .await;
    // Generate the client key inside the image (no ssh-keygen needed on the host).
    exec_out(
        &c,
        "mkdir -p /config/.ssh && ssh-keygen -q -t ed25519 -N '' -f /tmp/id && cat /tmp/id.pub >> /config/.ssh/authorized_keys \
         && ssh-keygen -q -t rsa -b 2048 -N '' -f /tmp/id_rsa && cat /tmp/id_rsa.pub >> /config/.ssh/authorized_keys \
         && chown -R crustdb /config/.ssh && chmod 700 /config/.ssh && chmod 600 /config/.ssh/authorized_keys",
    )
    .await;
    // The image forbids TCP forwarding by default; the tunnel tests need it.
    exec_out(
        &c,
        "sed -i 's/^AllowTcpForwarding no/AllowTcpForwarding yes/' /config/sshd/sshd_config && kill -HUP $(pgrep -o sshd)",
    )
    .await;
    let key = exec_out(&c, "cat /tmp/id").await;
    let host_public_key = exec_out(&c, "cat /config/ssh_host_keys/ssh_host_ed25519_key.pub")
        .await
        .trim()
        .to_string();
    let rsa_key = exec_out(&c, "cat /tmp/id_rsa").await;
    let host_rsa_public_key = exec_out(&c, "cat /config/ssh_host_keys/ssh_host_rsa_key.pub")
        .await
        .trim()
        .to_string();
    let dir = tempfile::tempdir().unwrap();
    let write_key = |name: &str, key: String| {
        let path = dir.path().join(name);
        std::fs::write(&path, key).unwrap();
        std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600))
            .unwrap();
        path
    };
    let client_key_path = write_key("id_ed25519", key);
    let rsa_client_key_path = write_key("id_rsa", rsa_key);
    let (host, port) = endpoint(&c, 2222).await;
    SshContainer {
        host,
        port,
        user: "crustdb".into(),
        password: PASSWORD.into(),
        alias,
        host_public_key,
        client_key_path,
        host_rsa_public_key,
        rsa_client_key_path,
        _dir: dir,
        _guard: c,
    }
}
