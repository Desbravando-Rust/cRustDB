#![allow(clippy::unwrap_used, clippy::expect_used, clippy::err_expect)] // Box<dyn Session> is not Debug

use std::path::Path;
use std::time::Duration;

use crustdb_core::dialect::{Dialect, build_table_query};
use crustdb_core::driver::{
    ConnectTarget, Driver, ExecMode, Session, SessionPurpose, TlsConfig, TlsMode,
};
use crustdb_core::error::ErrorKind;
use crustdb_core::model::{Cell, ObjectKind, ObjectRef, Product, TypeCategory};
use crustdb_core::query::{Filter, Operator, TableQuery};
use crustdb_sqlite::{SqliteDialect, SqliteDriver};
use tempfile::NamedTempFile;

async fn connect(path: &Path) -> Box<dyn Session> {
    let t = ConnectTarget {
        host: String::new(),
        port: 0,
        connect_addr: None,
        user: String::new(),
        password: None,
        database: Some(path.to_str().unwrap().into()),
        tls: TlsConfig {
            mode: TlsMode::Disable,
            ca_path: None,
            client_cert_path: None,
            client_key_path: None,
        },
        connect_timeout: Duration::from_secs(5),
    };
    SqliteDriver
        .connect(&t, SessionPurpose::Editor)
        .await
        .unwrap()
}

async fn run(s: &mut Box<dyn Session>, sql: &str) {
    s.start(sql, ExecMode::Other).await.unwrap();
    s.finish().await.unwrap();
}

const FIXTURE: &[&str] = &[
    "CREATE TABLE users(id INTEGER PRIMARY KEY, name TEXT NOT NULL, score REAL DEFAULT 0)",
    "CREATE VIEW v AS SELECT id FROM users",
    "CREATE TRIGGER tr AFTER INSERT ON users BEGIN SELECT 1; END",
];

fn obj(name: &str, kind: ObjectKind) -> ObjectRef {
    ObjectRef {
        database: "main".into(),
        schema: None,
        name: name.into(),
        kind,
    }
}

async fn names(s: &mut Box<dyn Session>, kind: ObjectKind) -> Vec<String> {
    SqliteDriver
        .introspector()
        .objects(s.as_mut(), "main", None, kind)
        .await
        .unwrap()
        .into_iter()
        .map(|o| o.name)
        .collect()
}

#[tokio::test]
async fn lists_main_tables_views_columns() {
    // Arrange
    let f = NamedTempFile::new().unwrap();
    let mut s = connect(f.path()).await;

    // Act & Assert
    assert_eq!(s.server_version().product, Product::Sqlite);
    assert_eq!(s.server_version().major, 3);
    for sql in FIXTURE {
        run(&mut s, sql).await;
    }
    let i = SqliteDriver.introspector();
    assert_eq!(i.databases(s.as_mut()).await.unwrap(), ["main"]);
    assert!(i.schemas(s.as_mut(), "main").await.unwrap().is_empty());
    assert_eq!(names(&mut s, ObjectKind::Table).await, ["users"]);
    assert_eq!(names(&mut s, ObjectKind::View).await, ["v"]);
    assert_eq!(names(&mut s, ObjectKind::Trigger).await, ["tr"]);
    let e = i
        .objects(s.as_mut(), "main", None, ObjectKind::Function)
        .await
        .err()
        .unwrap();
    assert_eq!(e.kind, ErrorKind::Unsupported);

    let cols = i
        .columns(s.as_mut(), &obj("users", ObjectKind::Table))
        .await
        .unwrap();
    let got: Vec<_> = cols
        .iter()
        .map(|c| {
            (
                c.meta.name.as_str(),
                c.meta.category,
                c.meta.nullable,
                c.meta.is_primary_key,
                c.default.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("id", TypeCategory::Integer, Some(true), true, None),
            ("name", TypeCategory::Text, Some(false), false, None),
            ("score", TypeCategory::Float, Some(true), false, Some("0")),
        ]
    );
}

#[tokio::test]
async fn ddl_returns_create_sql() {
    // Arrange
    let f = NamedTempFile::new().unwrap();
    let mut s = connect(f.path()).await;
    for sql in FIXTURE {
        run(&mut s, sql).await;
    }

    // Act & Assert
    let i = SqliteDriver.introspector();
    let ddl = i
        .ddl(s.as_mut(), &obj("users", ObjectKind::Table))
        .await
        .unwrap();
    assert!(ddl.starts_with("CREATE TABLE users("), "{ddl}");
    let ddl = i
        .ddl(s.as_mut(), &obj("v", ObjectKind::View))
        .await
        .unwrap();
    assert!(ddl.starts_with("CREATE VIEW v AS"), "{ddl}");
}

#[tokio::test]
async fn quote_ident_handles_double_quote() {
    // Arrange
    let f = NamedTempFile::new().unwrap();
    let mut s = connect(f.path()).await;
    let d = SqliteDialect;
    let (t, c) = (d.quote_ident("a\"b"), d.quote_ident("c\"d"));
    run(&mut s, &format!("CREATE TABLE {t}({c} TEXT)")).await;

    // Act & Assert
    run(&mut s, &format!("INSERT INTO {t} VALUES ('Hello'), ('x')")).await;

    let o = obj("a\"b", ObjectKind::Table);
    let cols = SqliteDriver
        .introspector()
        .columns(s.as_mut(), &o)
        .await
        .unwrap();
    assert_eq!(cols[0].meta.name, "c\"d");
    let q = TableQuery {
        object: o,
        columns: cols.into_iter().map(|c| c.meta).collect(),
        filters: vec![Filter {
            column: "c\"d".into(),
            op: Operator::Contains,
            values: vec!["ELL".into()],
        }],
        sorts: vec![],
        limit: Some(10),
        offset: 0,
        after: None,
    };
    let b = build_table_query(&d, &q).unwrap();
    s.start_params(&b.sql, &b.params).await.unwrap();
    let r = s.fetch(10).await.unwrap();
    s.finish().await.unwrap();
    assert_eq!(r.rows.len(), 1, "{}", b.sql);
    assert!(matches!(&r.rows[0][0], Cell::Text { value, .. } if &**value == "Hello"));
}
