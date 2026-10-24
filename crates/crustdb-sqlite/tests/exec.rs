#![allow(clippy::unwrap_used, clippy::expect_used, clippy::err_expect)] // Box<dyn Session> is not Debug

use std::path::Path;
use std::time::Duration;

use crustdb_core::driver::{
    ConnectTarget, Driver, ExecMode, Param, Session, SessionPurpose, TlsConfig, TlsMode,
};
use crustdb_core::error::ErrorKind;
use crustdb_core::model::{Cell, TypeCategory};
use crustdb_sqlite::SqliteDriver;
use tempfile::NamedTempFile;

fn target(path: &Path) -> ConnectTarget {
    ConnectTarget {
        host: String::new(),
        port: 0,
        connect_addr: None,
        user: String::new(),
        password: None,
        database: Some(path.to_str().unwrap().into()),
        tls: TlsConfig {
            mode: TlsMode::Disable,
            ca_path: None,
        },
        connect_timeout: Duration::from_secs(5),
    }
}

/// An empty file is a valid empty SQLite database.
async fn open(path: &Path, p: SessionPurpose) -> Box<dyn Session> {
    SqliteDriver.connect(&target(path), p).await.unwrap()
}

fn txt(c: &Cell) -> &str {
    match c {
        Cell::Text { value, .. } => value,
        other => panic!("not text: {other:?}"),
    }
}

/// start + fetch everything + finish, cells as text.
async fn all(s: &mut Box<dyn Session>, sql: &str) -> Vec<Vec<String>> {
    s.start(sql, ExecMode::Other).await.unwrap();
    let f = s.fetch(1_000_000).await.unwrap();
    s.finish().await.unwrap();
    f.rows
        .iter()
        .map(|r| r.iter().map(|c| txt(c).to_string()).collect())
        .collect()
}

async fn affected(s: &mut Box<dyn Session>, sql: &str) -> Option<u64> {
    s.start(sql, ExecMode::Other).await.unwrap();
    s.finish().await.unwrap().affected_rows
}

const SEED: &str = "WITH RECURSIVE c(x) AS \
    (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 25) INSERT INTO t SELECT x FROM c";

#[tokio::test]
async fn select_streams_rows_in_pages() {
    // Arrange
    let f = NamedTempFile::new().unwrap();
    let mut s = open(f.path(), SessionPurpose::Editor).await;
    affected(&mut s, "CREATE TABLE t(n INTEGER)").await;

    // Act & Assert
    assert_eq!(affected(&mut s, SEED).await, Some(25));

    let h = s
        .start("SELECT n FROM t ORDER BY n", ExecMode::Read)
        .await
        .unwrap();
    assert_eq!(h.columns[0].name, "n");
    assert_eq!(h.columns[0].category, TypeCategory::Integer);
    assert!(s.has_active_result());
    let a = s.fetch(10).await.unwrap();
    assert_eq!((a.rows.len(), a.exhausted), (10, false));
    assert_eq!(txt(&a.rows[0][0]), "1");
    let b = s.fetch(10).await.unwrap();
    assert_eq!((b.rows.len(), b.exhausted), (10, false));
    assert_eq!(txt(&b.rows[0][0]), "11");
    let c = s.fetch(10).await.unwrap();
    assert_eq!((c.rows.len(), c.exhausted), (5, true));
    assert!(!s.has_active_result());
    assert_eq!(s.finish().await.unwrap().affected_rows, None);

    // A paused result is discarded by the next command (D37).
    s.start("SELECT n FROM t", ExecMode::Read).await.unwrap();
    s.fetch(1).await.unwrap();
    assert_eq!(all(&mut s, "SELECT count(*) FROM t").await, [["25"]]);
    s.close().await;
}

#[tokio::test]
async fn dml_reports_affected_rows() {
    // Arrange
    let f = NamedTempFile::new().unwrap();
    let mut s = open(f.path(), SessionPurpose::Editor).await;

    // Act & Assert
    assert_eq!(affected(&mut s, "CREATE TABLE t(n INTEGER)").await, Some(0));
    assert_eq!(affected(&mut s, SEED).await, Some(25));
    assert_eq!(
        affected(&mut s, "UPDATE t SET n = n + 1 WHERE n > 20").await,
        Some(5)
    );
    // DDL after DML reports 0, not the previous DML count.
    assert_eq!(affected(&mut s, "CREATE TABLE u(n INTEGER)").await, Some(0));
    assert_eq!(affected(&mut s, "DELETE FROM t").await, Some(25));
}

#[tokio::test]
async fn start_params_binds_null_and_text() {
    // Arrange
    let f = NamedTempFile::new().unwrap();
    let mut s = open(f.path(), SessionPurpose::Editor).await;
    s.start_params(
        "SELECT ?1 IS NULL, ?2",
        &[Param(None), Param(Some("x'y".into()))],
    )
    .await
    .unwrap();
    let r = s.fetch(10).await.unwrap();

    // Act
    s.finish().await.unwrap();

    // Assert
    assert_eq!(txt(&r.rows[0][0]), "1");
    assert_eq!(txt(&r.rows[0][1]), "x'y");
}

#[tokio::test]
async fn read_only_purpose_rejects_write() {
    // Arrange
    let f = NamedTempFile::new().unwrap();
    let mut s = open(f.path(), SessionPurpose::Editor).await;
    affected(&mut s, "CREATE TABLE t(n INTEGER)").await;

    // Act & Assert
    s.close().await;

    let mut ro = open(f.path(), SessionPurpose::Metadata).await;
    let e = ro
        .start("INSERT INTO t VALUES (1)", ExecMode::Other)
        .await
        .err()
        .unwrap();
    assert!(matches!(e.kind, ErrorKind::Query { .. }), "{e:?}");
    assert_eq!(all(&mut ro, "SELECT count(*) FROM t").await, [["0"]]);

    // The file is never created.
    let missing = f.path().with_extension("missing");
    let e = SqliteDriver
        .connect(&target(&missing), SessionPurpose::Editor)
        .await
        .err()
        .unwrap();
    assert_eq!(e.kind, ErrorKind::Connect);
    assert!(!missing.exists());
}

#[tokio::test]
async fn cancel_interrupts_long_query() {
    // Arrange
    let f = NamedTempFile::new().unwrap();
    let mut s = open(f.path(), SessionPurpose::Editor).await;
    let h = s.cancel_handle();
    let mut q = tokio::spawn(async move {
        let r = async {
            s.start(
                "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 1000000000) \
                 SELECT count(*) FROM c",
                ExecMode::Read,
            )
            .await?;
            s.fetch(10).await
        }
        .await;
        (s, r)
    });

    // Act & Assert
    // An interrupt before the first step is a no-op, so repeat it until the query ends.
    let (mut s, r) = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            tokio::select! {
                r = &mut q => break r.unwrap(),
                () = tokio::time::sleep(Duration::from_millis(20)) => h.cancel().await.unwrap(),
            }
        }
    })
    .await
    .expect("cancel did not stop the query");
    assert_eq!(r.err().unwrap().kind, ErrorKind::Cancelled);
    s.finish().await.unwrap();
    assert_eq!(all(&mut s, "SELECT 1").await, [["1"]]);
}

#[tokio::test]
async fn multiple_statements_rejected() {
    // Arrange
    let f = NamedTempFile::new().unwrap();
    let mut s = open(f.path(), SessionPurpose::Editor).await;

    // Act & Assert
    let e = s
        .start("SELECT 1; SELECT 2", ExecMode::Other)
        .await
        .err()
        .unwrap();
    assert!(matches!(e.kind, ErrorKind::Query { .. }), "{e:?}");
    assert!(!s.has_active_result());
    assert_eq!(all(&mut s, "SELECT 2").await, [["2"]]);
}

#[tokio::test]
async fn comment_only_sql_is_noop() {
    // Arrange
    let f = NamedTempFile::new().unwrap();
    let mut s = open(f.path(), SessionPurpose::Editor).await;

    // Act & Assert
    for sql in ["", "-- just a comment", "/* block */"] {
        let h = s.start(sql, ExecMode::Other).await.unwrap();
        assert!(h.columns.is_empty(), "{sql:?}");
        assert_eq!(s.finish().await.unwrap().affected_rows, Some(0), "{sql:?}");
    }
}

#[tokio::test]
async fn update_cell_via_build_update_affects_one_row() {
    use crustdb_core::edit::build_update;
    use crustdb_core::model::{ColumnMeta, ObjectKind, ObjectRef};

    // Arrange
    let f = NamedTempFile::new().unwrap();
    let mut s = open(f.path(), SessionPurpose::Editor).await;
    affected(
        &mut s,
        "CREATE TABLE t(id INTEGER, k TEXT, v TEXT, PRIMARY KEY (id, k))",
    )
    .await;
    affected(&mut s, "INSERT INTO t VALUES (1, 'a', 'x'), (1, 'b', 'y')").await;
    let col = |name: &str, type_name: &str, category, pk| ColumnMeta {
        name: name.into(),
        type_name: type_name.into(),
        category,
        nullable: None,
        is_primary_key: pk,
    };
    let cols = vec![
        col("id", "INTEGER", TypeCategory::Integer, true),
        col("k", "TEXT", TypeCategory::Text, true),
        col("v", "TEXT", TypeCategory::Text, false),
    ];
    let object = ObjectRef {
        database: "main".into(),
        schema: None,
        name: "t".into(),
        kind: ObjectKind::Table,
    };
    let row = vec![
        Cell::text_capped("1"),
        Cell::text_capped("a"),
        Cell::text_capped("x"),
    ];
    let b = build_update(
        SqliteDriver.dialect(),
        &object,
        &cols,
        &row,
        &[(2, Some("z".into()))],
    )
    .unwrap();

    // Act & Assert
    s.start_params(&b.sql, &b.params).await.unwrap();
    assert_eq!(s.finish().await.unwrap().affected_rows, Some(1));
    assert_eq!(
        all(&mut s, "SELECT v FROM t ORDER BY k").await,
        vec![vec!["z".to_string()], vec!["y".to_string()]]
    );
}
