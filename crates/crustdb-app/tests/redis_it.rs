//! Docker integration test: `cargo test -p crustdb-app --test redis_it -- --include-ignored`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::err_expect)]

use std::sync::Arc;
use std::time::Duration;

use crustdb_app::runtime::{Command, Event, ResolvedSecrets, Runtime, WorkspaceId, drivers};
use crustdb_app::storage::config::ConnectionConfig;
use crustdb_core::driver::{TlsConfig, TlsMode};
use crustdb_core::model::{ConnectionId, DriverKind};
use crustdb_testkit::start_redis;
use secrecy::SecretString;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "docker"]
async fn redis_connects_and_lists_namespaces() {
    // Arrange
    let c = start_redis(None).await;
    let (rt, rx) = Runtime::start(Arc::new(|| {}), drivers());
    let ws = WorkspaceId(1);
    rt.send(Command::Connect {
        ws,
        config: ConnectionConfig {
            id: ConnectionId(uuid::Uuid::nil()),
            name: "it".into(),
            driver: DriverKind::Redis,
            host: c.host.clone(),
            port: c.port,
            user: String::new(),
            database: Some("0".into()),
            save_password: false,
            tls: TlsConfig {
                mode: TlsMode::Disable,
                ca_path: None,
                client_cert_path: None,
                client_key_path: None,
            },
            ssh: None,
            page_size: 300,
            editor_row_limit: 1000,
            safety_threshold: 1000,
            label: None,
            key_separator: None,
        },
        secrets: ResolvedSecrets {
            db: Some(SecretString::from(c.password.clone())),
            ..ResolvedSecrets::default()
        },
    });

    // Act
    let rx = tokio::task::spawn_blocking(move || {
        loop {
            match rx.recv_timeout(Duration::from_secs(60)).unwrap() {
                Event::Connected { .. } => break,
                Event::ConnectFailed { error, .. } => panic!("{error}"),
                _ => {}
            }
        }
        let req = rt.next_request_id();
        rt.send(Command::ListNamespaces { ws, req });
        let result = loop {
            if let Event::Namespaces { result, .. } =
                rx.recv_timeout(Duration::from_secs(60)).unwrap()
            {
                break result;
            }
        };
        rt.shutdown();
        result
    })
    .await
    .unwrap();

    // Assert
    assert!(rx.is_ok(), "{rx:?}");
}
