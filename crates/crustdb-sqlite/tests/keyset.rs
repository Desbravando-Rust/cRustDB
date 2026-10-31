#![allow(clippy::unwrap_used, clippy::expect_used, clippy::err_expect)] // Box<dyn Session> is not Debug

use std::time::Duration;

use crustdb_core::dialect::build_table_query;
use crustdb_core::driver::{
    ConnectTarget, Driver, ExecMode, Session, SessionPurpose, TlsConfig, TlsMode,
};
use crustdb_core::keyset::after;
use crustdb_core::model::{Cell, ObjectKind, ObjectRef};
use crustdb_core::query::{Sort, TableQuery};
use crustdb_sqlite::{SqliteDialect, SqliteDriver};
use tempfile::NamedTempFile;

const DRIVER: SqliteDriver = SqliteDriver;
const DIALECT: SqliteDialect = SqliteDialect;

fn obj() -> ObjectRef {
    ObjectRef {
        database: "main".into(),
        schema: None,
        name: "ks".into(),
        kind: ObjectKind::Table,
    }
}

const SIZE: u64 = 7;

fn sort(c: &str, descending: bool) -> Sort {
    Sort {
        column: c.into(),
        descending,
    }
}

fn text(r: &[Cell]) -> String {
    r.iter()
        .map(|c| match c {
            Cell::Text { value, .. } => value.to_string(),
            _ => "NULL".into(),
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Walks the whole table page by page, by keyset cursor or by `OFFSET`.
async fn walk(
    s: &mut Box<dyn Session>,
    o: &ObjectRef,
    sorts: &[Sort],
    keyset: bool,
) -> Vec<String> {
    let columns: Vec<_> = DRIVER
        .introspector()
        .columns(s.as_mut(), o)
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.meta)
        .collect();
    let mut q = TableQuery {
        object: o.clone(),
        columns,
        filters: vec![],
        sorts: sorts.to_vec(),
        limit: Some(SIZE + 1),
        offset: 0,
        after: None,
    };
    let mut out = Vec::new();
    loop {
        let b = build_table_query(&DIALECT, &q).unwrap();
        s.start_params(&b.sql, &b.params).await.unwrap();
        let f = s.fetch(1_000_000).await.unwrap();
        s.finish().await.unwrap();
        let more = f.rows.len() > SIZE as usize;
        let rows = &f.rows[..f.rows.len().min(SIZE as usize)];
        out.extend(rows.iter().map(|r| text(r)));
        if !more {
            return out;
        }
        q.offset += SIZE;
        if keyset {
            q.after = Some(after(&q, rows.last().unwrap()).expect("keyset-safe order"));
        }
    }
}

async fn check(s: &mut Box<dyn Session>, sorts: Vec<Sort>) {
    let o = obj();
    let by_offset = walk(s, &o, &sorts, false).await;
    let by_keyset = walk(s, &o, &sorts, true).await;
    assert_eq!(by_offset.len(), 50);
    assert_eq!(by_keyset, by_offset);
}

#[tokio::test]
async fn keyset_pages_match_offset_pages() {
    // Arrange
    let file = NamedTempFile::new().unwrap();
    let t = ConnectTarget {
        host: String::new(),
        port: 0,
        connect_addr: None,
        user: String::new(),
        password: None,
        database: Some(file.path().to_str().unwrap().into()),
        tls: TlsConfig {
            mode: TlsMode::Disable,
            ca_path: None,
            client_cert_path: None,
            client_key_path: None,
        },
        connect_timeout: Duration::from_secs(5),
    };
    let mut s = SqliteDriver
        .connect(&t, SessionPurpose::Editor)
        .await
        .unwrap();
    for sql in [
        "CREATE TABLE ks(id INTEGER NOT NULL, tag TEXT NOT NULL, v INTEGER NOT NULL, PRIMARY KEY(id, tag))",
        "WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 49) INSERT INTO ks SELECT i % 10, 'g' || (i / 10), i % 7 FROM n",
    ] {
        s.start(sql, ExecMode::Other).await.unwrap();
        s.finish().await.unwrap();
    }

    // Act & Assert
    check(&mut s, vec![sort("id", false), sort("tag", false)]).await;
    check(&mut s, vec![sort("id", true), sort("tag", false)]).await;
    check(&mut s, vec![sort("id", true), sort("tag", true)]).await;
    check(
        &mut s,
        vec![sort("v", false), sort("id", false), sort("tag", false)],
    )
    .await;
}
