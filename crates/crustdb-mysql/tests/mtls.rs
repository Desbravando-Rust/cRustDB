#![allow(clippy::unwrap_used, clippy::expect_used, clippy::err_expect)] // Box<dyn Session> is not Debug

use std::time::Duration;

use crustdb_core::driver::{ConnectTarget, Driver, SessionPurpose, TlsConfig, TlsMode};
use crustdb_core::error::ErrorKind;
use crustdb_mysql::MysqlDriver;
use crustdb_testkit::{MysqlContainer, TestCerts, certs_for, start_mysql_mtls};
use secrecy::SecretString;

fn target(db: &MysqlContainer, certs: &TestCerts, with_client_cert: bool) -> ConnectTarget {
    let dir = certs.dir.path();
    ConnectTarget {
        host: "localhost".into(),
        port: db.port,
        connect_addr: None,
        user: db.user.clone(),
        password: Some(SecretString::from(db.password.clone())),
        database: None,
        tls: TlsConfig {
            mode: TlsMode::VerifyFull,
            ca_path: Some(dir.join("ca.pem")),
            client_cert_path: with_client_cert.then(|| dir.join("client.crt")),
            client_key_path: with_client_cert.then(|| dir.join("client.key")),
        },
        connect_timeout: Duration::from_secs(10),
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn mtls_connects_with_client_cert() {
    // Arrange
    let certs = certs_for(&["localhost"]);
    let db = start_mysql_mtls(&certs).await;

    // Act
    let mut s = MysqlDriver
        .connect(&target(&db, &certs, true), SessionPurpose::Editor)
        .await
        .unwrap();

    // Assert
    s.ping().await.unwrap();
    s.close().await;
}

#[tokio::test]
#[ignore = "docker"]
async fn mtls_rejects_without_client_cert() {
    // Arrange
    let certs = certs_for(&["localhost"]);
    let db = start_mysql_mtls(&certs).await;

    // Act
    let err = MysqlDriver
        .connect(&target(&db, &certs, false), SessionPurpose::Editor)
        .await
        .err();

    // Assert
    let e = err.expect("must fail");
    assert_eq!(e.kind, ErrorKind::Auth, "{e:?}");
    assert!(e.message.contains("Access denied"), "{}", e.message);
}
