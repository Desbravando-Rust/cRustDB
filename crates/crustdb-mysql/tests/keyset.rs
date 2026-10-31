#![allow(clippy::unwrap_used, clippy::expect_used, clippy::err_expect)] // Box<dyn Session> is not Debug

use std::time::Duration;

use crustdb_core::dialect::build_table_query;
use crustdb_core::driver::{
    ConnectTarget, Driver, ExecMode, Session, SessionPurpose, TlsConfig, TlsMode,
};
use crustdb_core::keyset::after;
use crustdb_core::model::{Cell, ObjectKind, ObjectRef};
use crustdb_core::query::{Sort, TableQuery};
use crustdb_mysql::{MysqlDialect, MysqlDriver};
use crustdb_testkit::start_mysql;
use secrecy::SecretString;

const DRIVER: MysqlDriver = MysqlDriver;
const DIALECT: MysqlDialect = MysqlDialect;

fn obj() -> ObjectRef {
    ObjectRef {
        database: "ks_db".into(),
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
            // ENUM orders yield None here: that page then uses OFFSET.
            q.after = after(&q, rows.last().unwrap());
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
#[ignore = "docker"]
async fn keyset_pages_match_offset_pages() {
    // Arrange
    let db = start_mysql(None, None).await;
    let t = ConnectTarget {
        host: "localhost".into(),
        port: db.port,
        connect_addr: None,
        user: db.user.clone(),
        password: Some(SecretString::from(db.password.clone())),
        database: None,
        tls: TlsConfig {
            mode: TlsMode::Disable,
            ca_path: None,
            client_cert_path: None,
            client_key_path: None,
        },
        connect_timeout: Duration::from_secs(10),
    };
    let mut s = MysqlDriver
        .connect(&t, SessionPurpose::Editor)
        .await
        .unwrap();
    for sql in [
        "CREATE DATABASE ks_db",
        "CREATE TABLE ks_db.ks(id INT NOT NULL, tag VARCHAR(20) NOT NULL, v INT NOT NULL, e ENUM('b','a','c') NOT NULL, PRIMARY KEY(id, tag))",
        "INSERT INTO ks_db.ks SELECT i % 10, CONCAT('g', i DIV 10), i % 7, ELT(i % 3 + 1, 'a', 'b', 'c') FROM (WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 49) SELECT i FROM n) AS t",
    ] {
        s.start(sql, ExecMode::Other).await.unwrap();
        s.finish().await.unwrap();
    }

    // Act & Assert
    check(&mut s, vec![sort("id", false), sort("tag", false)]).await;
    check(&mut s, vec![sort("id", true), sort("tag", false)]).await;
    check(&mut s, vec![sort("id", true), sort("tag", true)]).await;
    // ENUM sorts by declaration index: must fall back to OFFSET.
    check(
        &mut s,
        vec![sort("e", false), sort("id", false), sort("tag", false)],
    )
    .await;
    check(
        &mut s,
        vec![sort("v", false), sort("id", false), sort("tag", false)],
    )
    .await;
}
