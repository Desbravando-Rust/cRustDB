#![allow(clippy::unwrap_used, clippy::expect_used, clippy::err_expect)] // Box<dyn Session> is not Debug

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use crustdb_core::driver::{ConnectTarget, Driver, ExecMode, SessionPurpose, TlsConfig, TlsMode};
use crustdb_core::error::ErrorKind;
use crustdb_core::model::Product;
use crustdb_mysql::MysqlDriver;
use crustdb_testkit::{MysqlContainer, TestCerts, certs_for, start_mysql};
use secrecy::SecretString;

fn target(db: &MysqlContainer, mode: TlsMode, certs: Option<&TestCerts>) -> ConnectTarget {
    ConnectTarget {
        host: "localhost".into(),
        port: db.port,
        connect_addr: None,
        user: db.user.clone(),
        password: Some(SecretString::from(db.password.clone())),
        database: None,
        tls: TlsConfig {
            mode,
            ca_path: certs.map(|c| c.dir.path().join("ca.pem")),
        },
        connect_timeout: Duration::from_secs(10),
    }
}

fn image() -> String {
    std::env::var("CRUSTDB_MYSQL_IMAGE").unwrap_or_else(|_| "mysql:8.4".into())
}

/// Servers that only offer DHE suites: rustls (ECDHE only) cannot negotiate TLS with them.
fn server_without_ecdhe() -> bool {
    let image = image();
    image.starts_with("mariadb:10.0") || image.starts_with("mysql:5.7")
}

#[tokio::test]
#[ignore = "docker"]
async fn connects_with_each_tls_mode() {
    if server_without_ecdhe() {
        return;
    }
    let certs = certs_for(&["localhost"]);
    let db = start_mysql(Some(&certs), None).await;
    for mode in [TlsMode::Require, TlsMode::VerifyCa, TlsMode::VerifyFull] {
        let mut s = MysqlDriver
            .connect(&target(&db, mode, Some(&certs)), SessionPurpose::Editor)
            .await
            .unwrap_or_else(|e| panic!("{mode:?}: {e}"));
        assert!(s.server_version().major >= 5);
        s.ping().await.unwrap();
        s.close().await;
    }
    let plain = start_mysql(None, None).await;
    let s = MysqlDriver
        .connect(
            &target(&plain, TlsMode::Disable, None),
            SessionPurpose::Editor,
        )
        .await
        .unwrap();
    s.close().await;
}

#[tokio::test]
#[ignore = "docker"]
async fn detects_mariadb_product() {
    let db = start_mysql(None, None).await;
    let s = MysqlDriver
        .connect(&target(&db, TlsMode::Disable, None), SessionPurpose::Editor)
        .await
        .unwrap();
    let v = s.server_version();
    let expected = if image().starts_with("mariadb") {
        Product::Mariadb
    } else {
        Product::Mysql
    };
    assert_eq!(v.product, expected, "{}", v.raw);
    assert!(v.major >= 5, "{}", v.raw);
}

#[tokio::test]
#[ignore = "docker"]
async fn verify_full_rejects_wrong_hostname() {
    let certs = certs_for(&["other.test"]);
    let db = start_mysql(Some(&certs), None).await;
    let err = MysqlDriver
        .connect(
            &target(&db, TlsMode::VerifyFull, Some(&certs)),
            SessionPurpose::Editor,
        )
        .await
        .err()
        .expect("must fail");
    assert_eq!(err.kind, ErrorKind::Tls, "{err}");
}

#[tokio::test]
#[ignore = "docker"]
async fn verify_ca_is_fail_closed_on_mysql() {
    // mysql_async cannot skip the hostname check with current rustls: VerifyCa == VerifyFull.
    let certs = certs_for(&["other.test"]);
    let db = start_mysql(Some(&certs), None).await;
    let err = MysqlDriver
        .connect(
            &target(&db, TlsMode::VerifyCa, Some(&certs)),
            SessionPurpose::Editor,
        )
        .await
        .err()
        .expect("must fail");
    assert_eq!(err.kind, ErrorKind::Tls, "{err}");
}

#[tokio::test]
#[ignore = "docker"]
async fn verify_ca_rejects_untrusted_ca() {
    let certs = certs_for(&["other.test"]);
    let foreign = certs_for(&["other.test"]);
    let db = start_mysql(Some(&certs), None).await;
    let err = MysqlDriver
        .connect(
            &target(&db, TlsMode::VerifyCa, Some(&foreign)),
            SessionPurpose::Editor,
        )
        .await
        .err()
        .expect("must fail");
    assert_eq!(err.kind, ErrorKind::Tls, "{err}");
}

/// Only meaningful against servers without ECDHE (rustls cannot negotiate with them).
#[tokio::test]
#[ignore = "docker"]
async fn tls_rejects_server_without_tls12() {
    if !server_without_ecdhe() {
        return;
    }
    let certs = certs_for(&["localhost"]);
    let db = start_mysql(Some(&certs), None).await;
    let err = MysqlDriver
        .connect(
            &target(&db, TlsMode::Require, Some(&certs)),
            SessionPurpose::Editor,
        )
        .await
        .err()
        .expect("must fail");
    assert_eq!(err.kind, ErrorKind::Tls, "{err}");
    // these servers answer with a HandshakeFailure alert (no common cipher), not a version alert,
    // so the D39 text does not apply; only the kind is asserted.
}

#[tokio::test]
#[ignore = "docker"]
async fn metadata_session_rejects_writes() {
    let db = start_mysql(None, None).await;
    let t = target(&db, TlsMode::Disable, None);
    let mut s = MysqlDriver
        .connect(&t, SessionPurpose::Metadata)
        .await
        .unwrap();
    let err = s
        .start("CREATE TABLE mysql.x (a int)", ExecMode::Other)
        .await
        .err()
        .expect("must fail");
    assert!(
        matches!(&err.kind, ErrorKind::Query { code: Some(c), .. } if c == "1792"),
        "{err:?}"
    );
    s.close().await;
    // Editor sessions can write.
    let mut s = MysqlDriver
        .connect(&t, SessionPurpose::Editor)
        .await
        .unwrap();
    s.start("CREATE TEMPORARY TABLE mysql.x (a int)", ExecMode::Other)
        .await
        .unwrap();
    s.finish().await.unwrap();
}

#[tokio::test]
#[ignore = "docker"]
async fn auth_failure_maps_to_auth_kind() {
    let db = start_mysql(None, None).await;
    let mut t = target(&db, TlsMode::Disable, None);
    t.password = Some(SecretString::from("wrong-pw-xyz"));
    let err = MysqlDriver
        .connect(&t, SessionPurpose::Editor)
        .await
        .err()
        .expect("must fail");
    assert_eq!(err.kind, ErrorKind::Auth, "{err}");
    assert!(!err.message.contains("wrong-pw-xyz"));
}

#[tokio::test]
async fn connect_timeout_after_10s() {
    // Accepts (kernel backlog) but never answers.
    let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let t = ConnectTarget {
        host: "127.0.0.1".into(),
        port: silent.local_addr().unwrap().port(),
        connect_addr: None,
        user: "u".into(),
        password: None,
        database: None,
        tls: TlsConfig {
            mode: TlsMode::Disable,
            ca_path: None,
        },
        connect_timeout: Duration::from_secs(1),
    };
    let start = Instant::now();
    let err = MysqlDriver
        .connect(&t, SessionPurpose::Editor)
        .await
        .err()
        .expect("must fail");
    assert_eq!(err.kind, ErrorKind::Timeout, "{err}");
    assert!(start.elapsed() < Duration::from_secs(3));
    drop(silent);
}

#[tokio::test]
#[ignore = "docker"]
async fn hostname_override_used_with_tunnel_addr() {
    if server_without_ecdhe() {
        return;
    }
    let certs = certs_for(&["localhost"]);
    let db = start_mysql(Some(&certs), None).await;
    let mut t = target(&db, TlsMode::VerifyFull, Some(&certs));
    t.connect_addr = Some(SocketAddr::from(([127, 0, 0, 1], db.port)));
    t.port = 1; // must be ignored in favour of connect_addr
    let mut s = MysqlDriver
        .connect(&t, SessionPurpose::Editor)
        .await
        .unwrap();
    s.ping().await.unwrap();
}

#[tokio::test]
#[ignore = "docker"]
async fn cancel_handle_cancels_running_query() {
    let db = start_mysql(None, None).await;
    let mut s = MysqlDriver
        .connect(&target(&db, TlsMode::Disable, None), SessionPurpose::Editor)
        .await
        .unwrap();
    let h = s.cancel_handle();
    let q = tokio::spawn(async move {
        // Long cross join: a killed SLEEP() returns normally, a killed scan errors.
        let r = async {
            s.start(
                "SELECT COUNT(*) FROM information_schema.columns a, information_schema.columns b, information_schema.columns c",
                ExecMode::Other,
            )
            .await?;
            s.fetch(10).await
        }
        .await;
        (s, r)
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    h.cancel().await.unwrap();
    let (_s, r) = q.await.unwrap();
    assert_eq!(r.err().unwrap().kind, ErrorKind::Cancelled);
}
