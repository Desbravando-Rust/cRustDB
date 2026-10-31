#![allow(clippy::unwrap_used, clippy::expect_used, clippy::err_expect)] // Box<dyn Session> is not Debug

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use crustdb_core::driver::{
    ConnectTarget, Driver, ExecMode, Param, Session, SessionPurpose, TlsConfig, TlsMode,
};
use crustdb_core::error::ErrorKind;
use crustdb_core::model::{Cell, Product, TypeCategory};
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
            client_cert_path: None,
            client_key_path: None,
        },
        connect_timeout: Duration::from_secs(10),
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn connects_with_each_tls_mode() {
    // Arrange
    let certs = certs_for(&["localhost"]);
    let pg = start_postgres(Some(&certs), None).await;

    // Act & Assert
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
    // Arrange
    let certs = certs_for(&["other.test"]);
    let pg = start_postgres(Some(&certs), None).await;

    // Act
    let err = PostgresDriver
        .connect(
            &target(&pg, TlsMode::VerifyFull, Some(&certs)),
            SessionPurpose::Editor,
        )
        .await
        .err()
        .expect("must fail");

    // Assert
    assert_eq!(err.kind, ErrorKind::Tls, "{err}");
}

#[tokio::test]
#[ignore = "docker"]
async fn verify_ca_accepts_wrong_hostname() {
    // Arrange
    let certs = certs_for(&["other.test"]);
    let pg = start_postgres(Some(&certs), None).await;

    // Act & Assert
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
    // Arrange
    let certs = certs_for(&["other.test"]);
    let foreign = certs_for(&["other.test"]);
    let pg = start_postgres(Some(&certs), None).await;

    // Act
    let err = PostgresDriver
        .connect(
            &target(&pg, TlsMode::VerifyCa, Some(&foreign)),
            SessionPurpose::Editor,
        )
        .await
        .err()
        .expect("must fail");

    // Assert
    assert_eq!(err.kind, ErrorKind::Tls, "{err}");
}

#[tokio::test]
#[ignore = "docker"]
async fn disable_against_tls_required_server_fails_cleanly() {
    // Arrange
    let certs = certs_for(&["localhost"]);
    let pg = start_postgres_tls_only(&certs).await;

    // Act
    let err = PostgresDriver
        .connect(&target(&pg, TlsMode::Disable, None), SessionPurpose::Editor)
        .await
        .err()
        .expect("must fail");

    // Assert
    assert_eq!(err.kind, ErrorKind::Auth, "{err:?}");
    assert!(!err.message.contains(&pg.password));
}

#[tokio::test]
#[ignore = "docker"]
async fn metadata_session_rejects_writes() {
    // Arrange
    let pg = start_postgres(None, None).await;
    let t = target(&pg, TlsMode::Disable, None);
    let mut s = PostgresDriver
        .connect(&t, SessionPurpose::Metadata)
        .await
        .unwrap();

    // Act & Assert
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
    // Arrange
    let pg = start_postgres(None, None).await;
    let mut t = target(&pg, TlsMode::Disable, None);
    t.password = Some(SecretString::from("wrong-pw-xyz"));

    // Act
    let err = PostgresDriver
        .connect(&t, SessionPurpose::Editor)
        .await
        .err()
        .expect("must fail");

    // Assert
    assert_eq!(err.kind, ErrorKind::Auth, "{err}");
    assert!(!err.message.contains("wrong-pw-xyz"));
}

#[tokio::test]
async fn connect_timeout_after_10s() {
    // Arrange
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
            client_cert_path: None,
            client_key_path: None,
        },
        connect_timeout: Duration::from_secs(1),
    };

    // Act
    // Outer guard: fails instead of hanging if the driver ignores connect_timeout.
    let start = Instant::now();
    let err = tokio::time::timeout(
        Duration::from_secs(10),
        PostgresDriver.connect(&t, SessionPurpose::Editor),
    )
    .await
    .expect("connect_timeout not applied")
    .err()
    .expect("must fail");
    let elapsed = start.elapsed();

    // Assert
    assert_eq!(err.kind, ErrorKind::Timeout, "{err}");
    assert!(elapsed >= t.connect_timeout, "{elapsed:?}");
    drop(silent);
}

#[tokio::test]
#[ignore = "docker"]
async fn hostaddr_used_with_real_tls_name() {
    // Arrange
    let certs = certs_for(&["localhost"]);
    let pg = start_postgres(Some(&certs), None).await;
    let mut t = target(&pg, TlsMode::VerifyFull, Some(&certs));
    t.connect_addr = Some(SocketAddr::from(([127, 0, 0, 1], pg.port)));
    t.port = 1; // must be ignored in favour of connect_addr

    // Act & Assert
    let mut s = PostgresDriver
        .connect(&t, SessionPurpose::Editor)
        .await
        .unwrap();
    s.ping().await.unwrap();
}

#[tokio::test]
#[ignore = "docker"]
async fn cancel_handle_cancels_running_query() {
    // Arrange
    let pg = start_postgres(None, None).await;
    let t = target(&pg, TlsMode::Disable, None);
    let mut s = PostgresDriver
        .connect(&t, SessionPurpose::Editor)
        .await
        .unwrap();
    let h = s.cancel_handle();

    // Act
    let q = tokio::spawn(async move {
        let r = async {
            s.start("SELECT pg_sleep(30)", ExecMode::Other).await?;
            s.fetch(10).await
        }
        .await;
        (s, r)
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    h.cancel().await.unwrap();
    let (_s, r) = q.await.unwrap();

    // Assert
    assert_eq!(r.err().unwrap().kind, ErrorKind::Cancelled);
}

async fn editor(pg: &PgContainer) -> Box<dyn Session> {
    PostgresDriver
        .connect(&target(pg, TlsMode::Disable, None), SessionPurpose::Editor)
        .await
        .unwrap()
}

fn txt(c: &Cell) -> &str {
    match c {
        Cell::Text { value, .. } => value,
        other => panic!("not text: {other:?}"),
    }
}

/// start + fetch everything + finish, cells as text.
async fn all(s: &mut Box<dyn Session>, sql: &str, mode: ExecMode) -> Vec<Vec<String>> {
    s.start(sql, mode).await.unwrap();
    let f = s.fetch(1_000_000).await.unwrap();
    s.finish().await.unwrap();
    f.rows
        .iter()
        .map(|r| r.iter().map(|c| txt(c).to_string()).collect())
        .collect()
}

fn rss_kb() -> u64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap();
    let l = s.lines().find(|l| l.starts_with("VmRSS:")).unwrap();
    l.split_whitespace().nth(1).unwrap().parse().unwrap()
}

#[tokio::test]
#[ignore = "docker"]
async fn read_cursor_load_more_bounded_memory() {
    // Arrange
    let pg = start_postgres(None, None).await;
    let mut s = editor(&pg).await;
    let before = rss_kb();

    // Act & Assert
    s.start(
        "SELECT g FROM generate_series(1, 50000000) g",
        ExecMode::Read,
    )
    .await
    .unwrap();
    let a = s.fetch(1000).await.unwrap();
    let b = s.fetch(1000).await.unwrap();
    assert!(!a.exhausted && !b.exhausted);
    assert_eq!((txt(&a.rows[0][0]), txt(&b.rows[0][0])), ("1", "1001"));
    assert!(s.has_active_result());
    s.finish().await.unwrap();
    assert!(!s.has_active_result());
    assert!(rss_kb().saturating_sub(before) < 50 * 1024);
}

#[tokio::test]
#[ignore = "docker"]
async fn zero_row_select_keeps_columns() {
    // Arrange
    let pg = start_postgres(None, None).await;
    let mut s = editor(&pg).await;

    // Act & Assert
    for mode in [ExecMode::Read, ExecMode::Other] {
        let h = s.start("SELECT 1 AS a WHERE false", mode).await.unwrap();
        assert_eq!(h.columns.len(), 1);
        assert_eq!(h.columns[0].name, "a");
        let f = s.fetch(10).await.unwrap();
        assert!(f.rows.is_empty() && f.exhausted);
        s.finish().await.unwrap();
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn pg_editor_columns_have_type_categories() {
    // Arrange
    let pg = start_postgres(None, None).await;
    let mut s = editor(&pg).await;

    // Act & Assert
    for mode in [ExecMode::Read, ExecMode::Other] {
        let h = s
            .start(
                "SELECT 1::int4 a, '{}'::jsonb b, '\\x01'::bytea c, NULL::text d",
                mode,
            )
            .await
            .unwrap();
        let cats: Vec<_> = h.columns.iter().map(|c| c.category).collect();
        assert_eq!(
            cats,
            [
                TypeCategory::Integer,
                TypeCategory::Json,
                TypeCategory::Binary,
                TypeCategory::Text
            ]
        );
        let f = s.fetch(10).await.unwrap();
        let r = &f.rows[0];
        assert_eq!(txt(&r[0]), "1");
        assert!(
            matches!(&r[2], Cell::Bytes { value, total_len: 1, truncated: false } if **value == [1])
        );
        assert_eq!(r[3], Cell::Null);
        s.finish().await.unwrap();
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn write_with_returning_is_drained_not_cancelled() {
    // Arrange
    let pg = start_postgres(None, None).await;
    let mut s = editor(&pg).await;
    all(&mut s, "CREATE TABLE w(id int)", ExecMode::Other).await;

    // Act & Assert
    s.start(
        "INSERT INTO w SELECT generate_series(1, 5000) RETURNING id",
        ExecMode::Other,
    )
    .await
    .unwrap();
    assert_eq!(s.fetch(10).await.unwrap().rows.len(), 10);
    let c = s.finish().await.unwrap();
    assert_eq!(c.affected_rows, Some(5000));
    assert_eq!(
        all(&mut s, "SELECT count(*) FROM w", ExecMode::Read).await[0][0],
        "5000"
    );
}

#[tokio::test]
#[ignore = "docker"]
async fn paused_cursor_finished_before_next_command() {
    // Arrange
    let pg = start_postgres(None, None).await;
    let mut s = editor(&pg).await;
    let mut other = editor(&pg).await;

    // Act & Assert
    s.start("SELECT g FROM generate_series(1, 100000) g", ExecMode::Read)
        .await
        .unwrap();
    s.fetch(10).await.unwrap();
    assert!(s.has_active_result());
    s.start("SELECT 2", ExecMode::Other).await.unwrap();
    assert_eq!(txt(&s.fetch(10).await.unwrap().rows[0][0]), "2");
    s.finish().await.unwrap();
    let idle = all(
        &mut other,
        "SELECT count(*) FROM pg_stat_activity WHERE state = 'idle in transaction'",
        ExecMode::Read,
    )
    .await;
    assert_eq!(idle[0][0], "0");
}

#[tokio::test]
#[ignore = "docker"]
async fn cursor_inside_user_transaction_keeps_it_open() {
    // Arrange
    let pg = start_postgres(None, None).await;
    let mut s = editor(&pg).await;
    let mut other = editor(&pg).await;
    all(&mut s, "BEGIN", ExecMode::Other).await;

    // Act & Assert
    s.start("SELECT 1", ExecMode::Read).await.unwrap();
    s.fetch(10).await.unwrap();
    s.finish().await.unwrap();
    let pid = all(&mut s, "SELECT pg_backend_pid()", ExecMode::Read).await[0][0].clone();
    let q = format!("SELECT state FROM pg_stat_activity WHERE pid = {pid}");
    assert_eq!(
        all(&mut other, &q, ExecMode::Read).await[0][0],
        "idle in transaction"
    );
    all(&mut s, "COMMIT", ExecMode::Other).await;
    assert_eq!(all(&mut other, &q, ExecMode::Read).await[0][0], "idle");
}

#[tokio::test]
#[ignore = "docker"]
async fn cancel_long_query() {
    // Arrange
    let pg = start_postgres(None, None).await;
    let mut s = editor(&pg).await;
    let h = s.cancel_handle();
    let t = Instant::now();

    // Act
    let q = tokio::spawn(async move {
        let r = async {
            s.start("SELECT pg_sleep(60)", ExecMode::Read).await?;
            s.fetch(10).await
        }
        .await;
        (s, r)
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    h.cancel().await.unwrap();
    let (mut s, r) = q.await.unwrap();

    // Assert
    assert_eq!(r.err().unwrap().kind, ErrorKind::Cancelled);
    assert!(t.elapsed() < Duration::from_millis(1500));
    s.finish().await.unwrap();
    assert_eq!(all(&mut s, "SELECT 1", ExecMode::Read).await[0][0], "1");
}

#[tokio::test]
#[ignore = "docker"]
async fn explain_rows_for_update_delete() {
    // Arrange
    let pg = start_postgres(None, None).await;
    let mut s = editor(&pg).await;
    all(
        &mut s,
        "CREATE TABLE t AS SELECT g AS id, g AS a FROM generate_series(1, 10000) g",
        ExecMode::Other,
    )
    .await;
    all(&mut s, "ANALYZE t", ExecMode::Other).await;

    // Act & Assert
    for sql in [
        "UPDATE t SET a = a WHERE id <= 5000",
        "DELETE FROM t WHERE id <= 5000",
    ] {
        let n = s.explain_rows(sql).await.unwrap().unwrap();
        assert!((2500..=10000).contains(&n), "{sql}: {n}");
    }
    assert_eq!(s.explain_rows("SELECT * FROM t").await.unwrap(), None);
}

#[tokio::test]
#[ignore = "docker"]
async fn pg_aborted_transaction_state() {
    // Arrange
    let pg = start_postgres(None, None).await;
    let mut s = editor(&pg).await;
    all(&mut s, "BEGIN", ExecMode::Other).await;

    // Act & Assert
    assert!(s.start("SELECT 1/0", ExecMode::Other).await.is_err());
    for mode in [ExecMode::Other, ExecMode::Read] {
        let e = s.start("SELECT 1", mode).await.err().unwrap();
        assert!(
            matches!(&e.kind, ErrorKind::Query { code: Some(c), .. } if c == "25P02"),
            "{e:?}"
        );
    }
    all(&mut s, "ROLLBACK", ExecMode::Other).await;
    assert_eq!(all(&mut s, "SELECT 1", ExecMode::Read).await[0][0], "1");
}

#[tokio::test]
#[ignore = "docker"]
async fn grid_caps_cell_transfer_at_1mib() {
    // Arrange
    let pg = start_postgres(None, None).await;
    let mut s = editor(&pg).await;

    // Act & Assert
    let h = s
        .start_params(
            "SELECT repeat('a', 50000000)::text AS big WHERE $1::text = 'x'",
            &[Param(Some("x".into()))],
        )
        .await
        .unwrap();
    assert_eq!(h.columns[0].name, "big");
    let f = s.fetch(10).await.unwrap();
    assert!(
        matches!(&f.rows[0][0], Cell::Text { value, truncated: true } if value.len() == 1 << 20)
    );
    s.finish().await.unwrap();
}

#[tokio::test]
#[ignore = "docker"]
async fn server_restart_during_paused_cursor_reports_error() {
    // Arrange
    let pg = start_postgres(None, None).await;
    let mut s = editor(&pg).await;
    s.start("SELECT g FROM generate_series(1, 100000) g", ExecMode::Read)
        .await
        .unwrap();
    s.fetch(10).await.unwrap();

    // Act
    pg.restart().await;
    let e = s.fetch(10).await.err().unwrap();

    // Assert
    assert_eq!(e.kind, ErrorKind::Connect, "{e:?}");
    let _ = s.finish().await;
}

async fn state_of(other: &mut Box<dyn Session>, pid: &str) -> String {
    let q = format!("SELECT state FROM pg_stat_activity WHERE pid = {pid}");
    all(other, &q, ExecMode::Read).await[0][0].clone()
}

#[tokio::test]
#[ignore = "docker"]
async fn rollback_to_savepoint_keeps_user_transaction() {
    // Arrange
    let pg = start_postgres(None, None).await;
    let mut s = editor(&pg).await;
    let mut other = editor(&pg).await;
    all(&mut s, "CREATE TABLE sp(a int)", ExecMode::Other).await;
    let pid = all(&mut s, "SELECT pg_backend_pid()", ExecMode::Read).await[0][0].clone();

    // Act & Assert
    for sql in [
        "BEGIN",
        "INSERT INTO sp VALUES (1)",
        "SAVEPOINT a",
        "ROLLBACK TO SAVEPOINT a",
    ] {
        all(&mut s, sql, ExecMode::Other).await;
    }
    s.start("SELECT 1", ExecMode::Read).await.unwrap();
    s.finish().await.unwrap();
    assert_eq!(state_of(&mut other, &pid).await, "idle in transaction");
    all(&mut s, "ROLLBACK", ExecMode::Other).await;
    assert_eq!(
        all(&mut s, "SELECT count(*) FROM sp", ExecMode::Read).await[0][0],
        "0"
    );
}

#[tokio::test]
#[ignore = "docker"]
async fn failing_prepare_in_user_tx_reports_real_error() {
    // Arrange
    let pg = start_postgres(None, None).await;
    let mut s = editor(&pg).await;
    all(&mut s, "BEGIN", ExecMode::Other).await;

    // Act
    let e = s
        .start("SELECT * FROM nope", ExecMode::Other)
        .await
        .err()
        .unwrap();

    // Assert
    assert!(
        matches!(&e.kind, ErrorKind::Query { code: Some(c), .. } if c == "42P01"),
        "{e:?}"
    );
}

#[tokio::test]
#[ignore = "docker"]
async fn failed_commit_ends_user_transaction() {
    // Arrange
    let pg = start_postgres(None, None).await;
    let mut s = editor(&pg).await;
    all(
        &mut s,
        "CREATE TABLE p(id int PRIMARY KEY)",
        ExecMode::Other,
    )
    .await;
    all(
        &mut s,
        "CREATE TABLE c(pid int REFERENCES p(id) DEFERRABLE INITIALLY DEFERRED)",
        ExecMode::Other,
    )
    .await;
    all(&mut s, "BEGIN", ExecMode::Other).await;
    all(&mut s, "INSERT INTO c VALUES (1)", ExecMode::Other).await;

    // Act & Assert
    assert!(s.start("COMMIT", ExecMode::Other).await.is_err());
    assert_eq!(all(&mut s, "SELECT 1", ExecMode::Read).await[0][0], "1");
}

#[tokio::test]
#[ignore = "docker"]
async fn failed_cursor_declare_leaves_no_open_transaction() {
    // Arrange
    let pg = start_postgres(None, None).await;
    let mut s = editor(&pg).await;
    let mut other = editor(&pg).await;
    let pid = all(&mut s, "SELECT pg_backend_pid()", ExecMode::Read).await[0][0].clone();

    // Act & Assert
    assert!(s.start("SELECT 1/0", ExecMode::Read).await.is_err());
    assert!(s.fetch(10).await.map(|f| f.rows.is_empty()).unwrap_or(true));
    s.finish().await.unwrap();
    assert_eq!(state_of(&mut other, &pid).await, "idle");
}
