#![allow(clippy::unwrap_used, clippy::expect_used, clippy::err_expect)] // Box<dyn Session> is not Debug

use std::time::Duration;

use crustdb_core::driver::{
    ConnectTarget, Driver, ExecMode, Session, SessionPurpose, TlsConfig, TlsMode,
};
use crustdb_core::edit::{build_delete, build_insert, build_update};
use crustdb_core::model::{Cell, ColumnMeta, ObjectKind, ObjectRef, TypeCategory};
use crustdb_postgres::PostgresDriver;
use crustdb_testkit::start_postgres;
use secrecy::SecretString;

async fn run(s: &mut Box<dyn Session>, sql: &str) {
    s.start(sql, ExecMode::Other).await.unwrap();
    s.finish().await.unwrap();
}

fn col(name: &str, type_name: &str, category: TypeCategory, pk: bool) -> ColumnMeta {
    ColumnMeta {
        name: name.into(),
        type_name: type_name.into(),
        category,
        nullable: None,
        is_primary_key: pk,
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn update_cell_via_build_update_affects_one_row() {
    // Arrange
    let pg = start_postgres(None, None).await;
    let target = ConnectTarget {
        host: "localhost".into(),
        port: pg.port,
        connect_addr: None,
        user: pg.user.clone(),
        password: Some(SecretString::from(pg.password.clone())),
        database: Some("postgres".into()),
        tls: TlsConfig {
            mode: TlsMode::Disable,
            ca_path: None,
            client_cert_path: None,
            client_key_path: None,
        },
        connect_timeout: Duration::from_secs(10),
    };
    let mut s = PostgresDriver
        .connect(&target, SessionPurpose::Editor)
        .await
        .unwrap();
    run(
        &mut s,
        "CREATE TABLE t(id integer, k text, v integer, PRIMARY KEY (id, k))",
    )
    .await;
    run(&mut s, "INSERT INTO t VALUES (1, 'a', 10), (1, 'b', 20)").await;
    let cols = vec![
        col("id", "integer", TypeCategory::Integer, true),
        col("k", "text", TypeCategory::Text, true),
        col("v", "integer", TypeCategory::Integer, false),
    ];
    let object = ObjectRef {
        database: "postgres".into(),
        schema: Some("public".into()),
        name: "t".into(),
        kind: ObjectKind::Table,
    };
    let row = vec![
        Cell::text_capped("1"),
        Cell::text_capped("a"),
        Cell::text_capped("10"),
    ];
    let dialect = PostgresDriver.dialect();

    // Act & Assert
    let b = build_update(dialect, &object, &cols, &row, &[(2, Some("42".into()))]).unwrap();
    s.start_params(&b.sql, &b.params).await.unwrap();
    assert_eq!(s.finish().await.unwrap().affected_rows, Some(1));
    let b = build_update(dialect, &object, &cols, &row, &[(2, None)]).unwrap();
    s.start_params(&b.sql, &b.params).await.unwrap();
    assert_eq!(s.finish().await.unwrap().affected_rows, Some(1));
    s.start(
        "SELECT count(*)::text FROM t WHERE v IS NULL",
        ExecMode::Read,
    )
    .await
    .unwrap();
    let f = s.fetch(10).await.unwrap();
    s.finish().await.unwrap();
    assert_eq!(f.rows[0][0], Cell::text_capped("1"));
    s.close().await;
}

#[tokio::test]
#[ignore = "docker"]
async fn insert_and_delete_via_build_affect_one_row() {
    // Arrange
    let pg = start_postgres(None, None).await;
    let target = ConnectTarget {
        host: "localhost".into(),
        port: pg.port,
        connect_addr: None,
        user: pg.user.clone(),
        password: Some(SecretString::from(pg.password.clone())),
        database: Some("postgres".into()),
        tls: TlsConfig {
            mode: TlsMode::Disable,
            ca_path: None,
        },
        connect_timeout: Duration::from_secs(10),
    };
    let mut s = PostgresDriver
        .connect(&target, SessionPurpose::Editor)
        .await
        .unwrap();
    run(&mut s, "CREATE TABLE t(id integer PRIMARY KEY, v text)").await;
    let cols = vec![
        col("id", "integer", TypeCategory::Integer, true),
        col("v", "text", TypeCategory::Text, false),
    ];
    let object = ObjectRef {
        database: "postgres".into(),
        schema: Some("public".into()),
        name: "t".into(),
        kind: ObjectKind::Table,
    };
    let dialect = PostgresDriver.dialect();
    let row = vec![Cell::text_capped("1"), Cell::Null];
    let ghost = vec![Cell::text_capped("99"), Cell::Null];

    // Act & Assert
    let b = build_insert(dialect, &object, &cols, &[(0, Some("1".into())), (1, None)]).unwrap();
    s.start_params(&b.sql, &b.params).await.unwrap();
    assert_eq!(s.finish().await.unwrap().affected_rows, Some(1));
    let b = build_delete(dialect, &object, &cols, &[&ghost]).unwrap();
    s.start_params(&b.sql, &b.params).await.unwrap();
    assert_eq!(s.finish().await.unwrap().affected_rows, Some(0));
    let b = build_delete(dialect, &object, &cols, &[&row]).unwrap();
    s.start_params(&b.sql, &b.params).await.unwrap();
    assert_eq!(s.finish().await.unwrap().affected_rows, Some(1));
}
