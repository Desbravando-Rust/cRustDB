#![allow(clippy::unwrap_used, clippy::expect_used, clippy::err_expect)] // Box<dyn Session> is not Debug

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use crustdb_core::driver::{ConnectTarget, Driver, ExecMode, SessionPurpose, TlsConfig, TlsMode};
use crustdb_core::error::ErrorKind;
use crustdb_core::model::Product;
use crustdb_postgres::PostgresDriver;
use crustdb_testkit::{PgContainer, TestCerts, certs_for, start_postgres, start_postgres_tls_only};
use secrecy::SecretString;

fn target(pg: &PgContainer, mode: TlsMode, certs: Option<&TestCerts>) -> ConnectTarget {
    ConnectTarget {
        host: "localhost".into(),
        port: pg.port,
        connect_addr: None,
        user: pg.user.clone(),
        password: Some(SecretString::from(pg.password.clone())),
        database: Some("postgres".into()),
        tls: TlsConfig {
            mode,
            ca_path: certs.map(|c| c.dir.path().join("ca.pem")),
        },
        connect_timeout: Duration::from_secs(10),
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn connects_with_each_tls_mode() {
    let certs = certs_for(&["localhost"]);
    let pg = start_postgres(Some(&certs), None).await;
    for mode in [TlsMode::Require, TlsMode::VerifyCa, TlsMode::VerifyFull] {
        let mut s = PostgresDriver
            .connect(&target(&pg, mode, Some(&certs)), SessionPurpose::Editor)
            .await
            .unwrap_or_else(|e| panic!("{mode:?}: {e}"));
        assert_eq!(s.server_version().product, Product::Postgres);
        assert!(s.server_version().major >= 12);
        s.ping().await.unwrap();
        s.close().await;
    }
    let pg_plain = start_postgres(None, None).await;
    let s = PostgresDriver
        .connect(
            &target(&pg_plain, TlsMode::Disable, None),
            SessionPurpose::Editor,
        )
        .await
        .unwrap();
    s.close().await;
}

#[tokio::test]
#[ignore = "docker"]
async fn verify_full_rejects_wrong_hostname() {
    let certs = certs_for(&["other.test"]);
    let pg = start_postgres(Some(&certs), None).await;
    let err = PostgresDriver
        .connect(
            &target(&pg, TlsMode::VerifyFull, Some(&certs)),
            SessionPurpose::Editor,
        )
        .await
        .err()
        .expect("must fail");
    assert_eq!(err.kind, ErrorKind::Tls, "{err}");
}

#[tokio::test]
#[ignore = "docker"]
async fn verify_ca_accepts_wrong_hostname() {
    let certs = certs_for(&["other.test"]);
    let pg = start_postgres(Some(&certs), None).await;
    let s = PostgresDriver
        .connect(
            &target(&pg, TlsMode::VerifyCa, Some(&certs)),
            SessionPurpose::Editor,
        )
        .await
        .unwrap();
    s.close().await;
}

#[tokio::test]
#[ignore = "docker"]
async fn verify_ca_rejects_untrusted_ca() {
    let certs = certs_for(&["other.test"]);
    let foreign = certs_for(&["other.test"]);
    let pg = start_postgres(Some(&certs), None).await;
    let err = PostgresDriver
        .connect(
            &target(&pg, TlsMode::VerifyCa, Some(&foreign)),
            SessionPurpose::Editor,
        )
        .await
        .err()
        .expect("must fail");
    assert_eq!(err.kind, ErrorKind::Tls, "{err}");
}

#[tokio::test]
#[ignore = "docker"]
async fn disable_against_tls_required_server_fails_cleanly() {
    let certs = certs_for(&["localhost"]);
    let pg = start_postgres_tls_only(&certs).await;
    let err = PostgresDriver
        .connect(&target(&pg, TlsMode::Disable, None), SessionPurpose::Editor)
        .await
        .err()
        .expect("must fail");
    assert_eq!(err.kind, ErrorKind::Auth, "{err:?}");
    assert!(!err.message.contains(&pg.password));
}

#[tokio::test]
#[ignore = "docker"]
async fn metadata_session_rejects_writes() {
    let pg = start_postgres(None, None).await;
    let t = target(&pg, TlsMode::Disable, None);
    let mut s = PostgresDriver
        .connect(&t, SessionPurpose::Metadata)
        .await
        .unwrap();
    let err = s
        .start("CREATE TABLE x()", ExecMode::Other)
        .await
        .err()
        .expect("must fail");
    assert!(
        matches!(&err.kind, ErrorKind::Query { code: Some(c), .. } if c == "25006"),
        "{err:?}"
    );
    s.close().await;
    // Editor sessions can write.
    let mut s = PostgresDriver
        .connect(&t, SessionPurpose::Editor)
        .await
        .unwrap();
    s.start("CREATE TEMP TABLE x()", ExecMode::Other)
        .await
        .unwrap();
    s.finish().await.unwrap();
}

#[tokio::test]
#[ignore = "docker"]
async fn auth_failure_maps_to_auth_kind() {
    let pg = start_postgres(None, None).await;
    let mut t = target(&pg, TlsMode::Disable, None);
    t.password = Some(SecretString::from("wrong-pw-xyz"));
    let err = PostgresDriver
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
    let err = PostgresDriver
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
async fn hostaddr_used_with_real_tls_name() {
    let certs = certs_for(&["localhost"]);
    let pg = start_postgres(Some(&certs), None).await;
    let mut t = target(&pg, TlsMode::VerifyFull, Some(&certs));
    t.connect_addr = Some(SocketAddr::from(([127, 0, 0, 1], pg.port)));
    t.port = 1; // must be ignored in favour of connect_addr
    let mut s = PostgresDriver
        .connect(&t, SessionPurpose::Editor)
        .await
        .unwrap();
    s.ping().await.unwrap();
}

#[tokio::test]
#[ignore = "docker"]
async fn cancel_handle_cancels_running_query() {
    let pg = start_postgres(None, None).await;
    let t = target(&pg, TlsMode::Disable, None);
    let mut s = PostgresDriver
        .connect(&t, SessionPurpose::Editor)
        .await
        .unwrap();
    let h = s.cancel_handle();
    let q = tokio::spawn(async move {
        let r = s.start("SELECT pg_sleep(30)", ExecMode::Other).await;
        (s, r)
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    h.cancel().await.unwrap();
    let (_s, r) = q.await.unwrap();
    assert!(
        matches!(&r.err().unwrap().kind, ErrorKind::Query { code: Some(c), .. } if c == "57014")
    );
}
