#![allow(clippy::unwrap_used, clippy::expect_used)] // Box<dyn Session> is not Debug

use std::time::Duration;

use crustdb_core::dialect::build_table_query;
use crustdb_core::driver::{
    ConnectTarget, Driver, ExecMode, Session, SessionPurpose, TlsConfig, TlsMode,
};
use crustdb_core::model::{Cell, ObjectKind, ObjectRef, TypeCategory};
use crustdb_core::query::{Filter, Operator, Sort, TableQuery};
use crustdb_postgres::{PgDialect, PostgresDriver};
use crustdb_testkit::{PgContainer, start_postgres};
use secrecy::SecretString;

const FIXTURE: &str = include_str!("fixtures/pg_fixture.sql");

async fn connect(pg: &PgContainer) -> Box<dyn Session> {
    let t = ConnectTarget {
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
    PostgresDriver
        .connect(&t, SessionPurpose::Editor)
        .await
        .unwrap()
}

async fn run(s: &mut Box<dyn Session>, sql: &str) {
    s.start(sql, ExecMode::Other).await.unwrap();
    s.finish().await.unwrap();
}

/// Container + editor session with the fixture applied.
async fn setup() -> (PgContainer, Box<dyn Session>) {
    let pg = start_postgres(None, None).await;
    let mut s = connect(&pg).await;
    run(&mut s, FIXTURE).await;
    (pg, s)
}

fn obj(schema: &str, name: &str, kind: ObjectKind) -> ObjectRef {
    ObjectRef {
        database: "postgres".into(),
        schema: Some(schema.into()),
        name: name.into(),
        kind,
    }
}

async fn names(s: &mut Box<dyn Session>, schema: &str, kind: ObjectKind) -> Vec<String> {
    PostgresDriver
        .introspector()
        .objects(s.as_mut(), "postgres", Some(schema), kind)
        .await
        .unwrap()
        .into_iter()
        .map(|o| o.name)
        .collect()
}

#[tokio::test]
#[ignore = "docker"]
async fn lists_tree_levels_per_driver() {
    // Arrange
    let (_pg, mut s) = setup().await;
    let i = PostgresDriver.introspector();

    // Act & Assert
    let dbs = i.databases(s.as_mut()).await.unwrap();
    assert!(dbs.contains(&"postgres".into()), "{dbs:?}");
    assert!(!dbs.iter().any(|d| d.starts_with("template")), "{dbs:?}");

    let schemas = i.schemas(s.as_mut(), "postgres").await.unwrap();
    for want in ["public", "app", "pg_catalog", "information_schema"] {
        assert!(schemas.contains(&want.into()), "{want}: {schemas:?}");
    }
    assert!(
        !schemas.iter().any(|n| n.starts_with("pg_toast")),
        "{schemas:?}"
    );
    assert!(
        !schemas.iter().any(|n| n.starts_with("pg_temp")),
        "{schemas:?}"
    );

    use ObjectKind::*;
    let cases: [(&str, ObjectKind, &[&str]); 9] = [
        ("app", Table, &["customers", "orders"]),
        ("public", Table, &["events", "events_2024"]),
        ("app", View, &["order_view"]),
        ("app", MaterializedView, &["order_mv"]),
        ("app", Function, &["add_one", "touch"]),
        ("app", Procedure, &["noop"]),
        ("app", Trigger, &["orders_touch"]),
        ("app", Sequence, &["ticket_seq"]),
        ("app", Type, &["mood", "pair", "posint"]),
    ];
    for (schema, kind, want) in cases {
        let got = names(&mut s, schema, kind).await;
        for w in want {
            assert!(got.contains(&(*w).into()), "{kind:?} {w}: {got:?}");
        }
    }
    // Type folder excludes table row types; each folder only holds its kind.
    let types = names(&mut s, "app", Type).await;
    assert!(!types.contains(&"customers".into()), "{types:?}");
    assert!(
        !names(&mut s, "app", Table)
            .await
            .contains(&"order_view".into())
    );
    let t = i
        .objects(s.as_mut(), "postgres", Some("app"), Table)
        .await
        .unwrap();
    assert_eq!(t[0], obj("app", "customers", Table));
}

#[tokio::test]
#[ignore = "docker"]
async fn row_estimate_without_count() {
    // Arrange
    let (_pg, mut s) = setup().await;
    run(
        &mut s,
        "CREATE TABLE est_a AS SELECT g FROM generate_series(1, 1000) g;
         CREATE TABLE est_b AS SELECT g FROM generate_series(1, 100) g;
         ANALYZE est_a",
    )
    .await;
    let i = PostgresDriver.introspector();

    // Act & Assert
    let a = i
        .row_estimate(s.as_mut(), &obj("public", "est_a", ObjectKind::Table))
        .await
        .unwrap()
        .unwrap();
    assert!((900..=1100).contains(&a), "{a}");
    let b = i
        .row_estimate(s.as_mut(), &obj("public", "est_b", ObjectKind::Table))
        .await
        .unwrap();
    assert_eq!(b, None);
    let v = i
        .row_estimate(s.as_mut(), &obj("app", "order_view", ObjectKind::View))
        .await
        .unwrap();
    assert_eq!(v, None);
}

#[tokio::test]
#[ignore = "docker"]
async fn ddl_for_each_object_kind() {
    // Arrange
    let (_pg, mut s) = setup().await;
    use ObjectKind::*;
    let all = [
        obj("app", "customers", Table),
        obj("app", "orders", Table),
        obj("public", "events", Table),
        obj("app", "order_view", View),
        obj("app", "order_mv", MaterializedView),
        obj("app", "add_one", Function),
        obj("app", "noop", Procedure),
        obj("app", "orders_touch", Trigger),
        obj("app", "ticket_seq", Sequence),
        obj("app", "mood", Type),
        obj("app", "pair", Type),
        obj("app", "posint", Type),
    ];
    let i = PostgresDriver.introspector();

    // Act & Assert
    for o in &all {
        let ddl = i.ddl(s.as_mut(), o).await.unwrap();
        assert!(ddl.contains(&o.name), "{}: {ddl}", o.name);
    }
    let orders = i.ddl(s.as_mut(), &all[1]).await.unwrap();
    assert!(
        orders.contains("FOREIGN KEY") && orders.contains("orders_customer_idx"),
        "{orders}"
    );
    let ev = i.ddl(s.as_mut(), &all[2]).await.unwrap();
    assert!(ev.contains("PARTITION BY RANGE"), "{ev}");
    let part = i
        .ddl(s.as_mut(), &obj("public", "events_2024", Table))
        .await
        .unwrap();
    assert!(
        part.contains("PARTITION OF") && part.contains("FOR VALUES"),
        "{part}"
    );
    let f = i.ddl(s.as_mut(), &all[5]).await.unwrap();
    assert_eq!(f.matches("CREATE OR REPLACE FUNCTION").count(), 2, "{f}");
}

#[tokio::test]
#[ignore = "docker"]
async fn structure_lists_columns_indexes_fks() {
    // Arrange
    let (_pg, mut s) = setup().await;
    let i = PostgresDriver.introspector();

    // Act & Assert
    let cols = i
        .columns(s.as_mut(), &obj("app", "customers", ObjectKind::Table))
        .await
        .unwrap();
    let n: Vec<_> = cols.iter().map(|c| c.meta.name.as_str()).collect();
    assert_eq!(n, ["id", "name", "email", "mood"]);
    assert!(cols[0].meta.is_primary_key && !cols[1].meta.is_primary_key);
    assert_eq!(cols[0].meta.type_name, "integer");
    assert_eq!(cols[0].meta.category, TypeCategory::Integer);
    assert!(cols[0].default.as_deref().unwrap().contains("nextval"));
    assert_eq!(cols[0].meta.nullable, Some(false));
    assert_eq!(cols[2].meta.nullable, Some(true));
    assert_eq!(cols[1].comment.as_deref(), Some("full name"));
    assert_eq!(cols[3].meta.type_name, "app.mood");
    assert_eq!(cols[3].meta.category, TypeCategory::Other);

    let orders = obj("app", "orders", ObjectKind::Table);
    let idx = i.indexes(s.as_mut(), &orders).await.unwrap();
    let pk = idx.iter().find(|x| x.name == "orders_pkey").unwrap();
    assert!(pk.primary && pk.unique && pk.columns == ["id"]);
    let ci = idx
        .iter()
        .find(|x| x.name == "orders_customer_idx")
        .unwrap();
    assert!(!ci.unique && !ci.primary);
    assert_eq!(ci.columns, ["customer_id", "qty"]);
    assert_eq!(ci.method.as_deref(), Some("btree"));

    let fks = i.foreign_keys(s.as_mut(), &orders).await.unwrap();
    assert_eq!(fks.len(), 1);
    let fk = &fks[0];
    assert_eq!(fk.columns, ["customer_id"]);
    assert_eq!(fk.ref_table, obj("app", "customers", ObjectKind::Table));
    assert_eq!(fk.ref_columns, ["id"]);
    assert_eq!(
        (fk.on_update.as_str(), fk.on_delete.as_str()),
        ("RESTRICT", "CASCADE")
    );
}

type Page = Vec<Vec<String>>;

async fn page(
    s: &mut Box<dyn Session>,
    o: &ObjectRef,
    filters: Vec<Filter>,
    sorts: Vec<Sort>,
    page_no: u64,
    size: u64,
) -> Page {
    let columns = PostgresDriver
        .introspector()
        .columns(s.as_mut(), o)
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.meta)
        .collect();
    let q = TableQuery {
        object: o.clone(),
        columns,
        filters,
        sorts,
        limit: Some(size + 1),
        offset: page_no * size,
    };
    let b = build_table_query(&PgDialect, &q).unwrap();
    s.start_params(&b.sql, &b.params).await.unwrap();
    let f = s.fetch(1_000_000).await.unwrap();
    s.finish().await.unwrap();
    f.rows
        .iter()
        .map(|r| {
            r.iter()
                .map(|c| match c {
                    Cell::Text { value, .. } => value.to_string(),
                    _ => "NULL".into(),
                })
                .collect()
        })
        .collect()
}

fn f(column: &str, op: Operator, values: &[&str]) -> Filter {
    Filter {
        column: column.into(),
        op,
        values: values.iter().map(|v| (*v).into()).collect(),
    }
}

async fn grid_table(s: &mut Box<dyn Session>) -> ObjectRef {
    run(
        s,
        "CREATE TABLE grid AS SELECT g AS id, 'name' || g AS name, g % 5 AS score,
            CASE WHEN g % 6 = 0 THEN NULL WHEN g % 2 = 0 THEN '50% off' ELSE 'plain' END AS note,
            chr(65 + g % 3) AS tag
         FROM generate_series(1, 30) g",
    )
    .await;
    obj("public", "grid", ObjectKind::Table)
}

#[tokio::test]
#[ignore = "docker"]
async fn fetches_page_with_filters_and_sorts() {
    // Arrange
    let (_pg, mut s) = setup().await;
    let g = grid_table(&mut s).await;

    // Act & Assert
    let one = page(
        &mut s,
        &g,
        vec![f("id", Operator::Eq, &["7"])],
        vec![],
        0,
        10,
    )
    .await;
    assert_eq!(one, [["7", "name7", "2", "plain", "B"]]);
    let pct = page(
        &mut s,
        &g,
        vec![f("note", Operator::Contains, &["%"])],
        vec![],
        0,
        100,
    )
    .await;
    assert_eq!(pct.len(), 10);
    let tags = page(
        &mut s,
        &g,
        vec![f("tag", Operator::In, &["A", "B"])],
        vec![],
        0,
        100,
    )
    .await;
    assert_eq!(tags.len(), 20);
    let bt = page(
        &mut s,
        &g,
        vec![f("id", Operator::Between, &["5", "9"])],
        vec![],
        0,
        100,
    )
    .await;
    assert_eq!(bt.len(), 5);
    let nul = page(
        &mut s,
        &g,
        vec![f("note", Operator::IsNull, &[])],
        vec![],
        0,
        100,
    )
    .await;
    assert_eq!(nul.len(), 5);

    let sorts = vec![
        Sort {
            column: "score".into(),
            descending: false,
        },
        Sort {
            column: "name".into(),
            descending: true,
        },
    ];
    let mut expected: Vec<u32> = (1..=30).collect();
    expected.sort_by_key(|&g| (g % 5, std::cmp::Reverse(format!("name{g}"))));
    let ids = |p: &Page| {
        p.iter()
            .take(10)
            .map(|r| r[0].parse::<u32>().unwrap())
            .collect::<Vec<_>>()
    };
    let p0 = page(&mut s, &g, vec![], sorts.clone(), 0, 10).await;
    let p1 = page(&mut s, &g, vec![], sorts, 1, 10).await;
    assert_eq!(p0.len(), 11); // page size + 1 probe row
    assert_eq!(ids(&p0), expected[..10]);
    assert_eq!(ids(&p1), expected[10..20]);
}

#[tokio::test]
#[ignore = "docker"]
async fn numeric_sorts_are_not_lexicographic() {
    // Arrange
    let (_pg, mut s) = setup().await;
    let g = grid_table(&mut s).await;
    run(&mut s, "ALTER TABLE grid ADD PRIMARY KEY (id)").await;
    let ids = |p: &Page| {
        p.iter()
            .take(10)
            .map(|r| r[0].parse::<u32>().unwrap())
            .collect::<Vec<_>>()
    };
    let desc = vec![Sort {
        column: "id".into(),
        descending: true,
    }];

    // Act & Assert
    let d0 = page(&mut s, &g, vec![], desc, 0, 10).await;
    assert_eq!(ids(&d0), (21..=30).rev().collect::<Vec<_>>());
    // default sort = primary key
    let p0 = page(&mut s, &g, vec![], vec![], 0, 10).await;
    let p1 = page(&mut s, &g, vec![], vec![], 1, 10).await;
    assert_eq!(ids(&p0), (1..=10).collect::<Vec<_>>());
    assert_eq!(ids(&p1), (11..=20).collect::<Vec<_>>());
}

#[tokio::test]
#[ignore = "docker"]
async fn json_column_can_be_sorted() {
    // Arrange
    let (_pg, mut s) = setup().await;
    run(
        &mut s,
        "CREATE TABLE j (id int PRIMARY KEY, doc json);
         INSERT INTO j VALUES (1, '{\"b\":1}'), (2, '{\"a\":1}')",
    )
    .await;
    let o = obj("public", "j", ObjectKind::Table);
    let sorts = vec![Sort {
        column: "doc".into(),
        descending: false,
    }];

    // Act
    let rows = page(&mut s, &o, vec![], sorts, 0, 10).await;

    // Assert
    assert_eq!(rows[0][0], "2");
}

#[tokio::test]
#[ignore = "docker"]
async fn filter_value_injection_is_inert() {
    // Arrange
    let (_pg, mut s) = setup().await;
    let g = grid_table(&mut s).await;
    let evil = "'; DROP TABLE grid; --";

    // Act & Assert
    for op in [Operator::Eq, Operator::Contains] {
        let rows = page(&mut s, &g, vec![f("name", op, &[evil])], vec![], 0, 10).await;
        assert!(rows.is_empty());
    }
    let n = page(&mut s, &g, vec![], vec![], 0, 100).await;
    assert_eq!(n.len(), 30);
}

#[tokio::test]
#[ignore = "docker"]
async fn table_with_quoted_mixed_case_name_opens() {
    // Arrange
    let (_pg, mut s) = setup().await;
    run(
        &mut s,
        r#"CREATE SCHEMA "Ação";
           CREATE TABLE "Ação"."Weird ""Name""" ("Mixed Col" int, "Val" text);
           INSERT INTO "Ação"."Weird ""Name""" VALUES (1, 'x'), (2, 'y')"#,
    )
    .await;
    let i = PostgresDriver.introspector();

    // Act & Assert
    let tables = i
        .objects(s.as_mut(), "postgres", Some("Ação"), ObjectKind::Table)
        .await
        .unwrap();
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].name, r#"Weird "Name""#);
    let cols = i.columns(s.as_mut(), &tables[0]).await.unwrap();
    assert_eq!(cols[0].meta.name, "Mixed Col");
    let rows = page(
        &mut s,
        &tables[0],
        vec![f("Val", Operator::Eq, &["y"])],
        vec![],
        0,
        10,
    )
    .await;
    assert_eq!(rows, [["2", "y"]]);
    let ddl = i.ddl(s.as_mut(), &tables[0]).await.unwrap();
    assert!(ddl.contains(r#""Weird ""Name""""#), "{ddl}");
}
