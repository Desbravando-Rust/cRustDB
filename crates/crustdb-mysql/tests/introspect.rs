#![allow(clippy::unwrap_used, clippy::expect_used, clippy::err_expect)] // Box<dyn Session> is not Debug

use std::time::Duration;

use crustdb_core::dialect::build_table_query;
use crustdb_core::driver::{
    ConnectTarget, Driver, ExecMode, Session, SessionPurpose, TlsConfig, TlsMode,
};
use crustdb_core::model::{Cell, ObjectKind, ObjectRef, Product, TypeCategory};
use crustdb_core::query::{Filter, Operator, Sort, TableQuery};
use crustdb_mysql::{MysqlDialect, MysqlDriver};
use crustdb_testkit::{MysqlContainer, start_mysql};
use secrecy::SecretString;

const FIXTURE: &str = include_str!("fixtures/mysql_fixture.sql");

async fn connect(db: &MysqlContainer) -> Box<dyn Session> {
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
        },
        connect_timeout: Duration::from_secs(10),
    };
    MysqlDriver
        .connect(&t, SessionPurpose::Editor)
        .await
        .unwrap()
}

async fn run(s: &mut Box<dyn Session>, sql: &str) {
    s.start(sql, ExecMode::Other).await.unwrap();
    s.finish().await.unwrap();
}

/// Container + editor session with the fixture applied (one statement per blank-line block).
async fn setup() -> (MysqlContainer, Box<dyn Session>) {
    let db = start_mysql(None, None).await;
    let mut s = connect(&db).await;
    for stmt in FIXTURE.split("\n\n") {
        run(&mut s, stmt).await;
    }
    (db, s)
}

fn obj(db: &str, name: &str, kind: ObjectKind) -> ObjectRef {
    ObjectRef {
        database: db.into(),
        schema: None,
        name: name.into(),
        kind,
    }
}

fn app(name: &str, kind: ObjectKind) -> ObjectRef {
    obj("app", name, kind)
}

async fn names(s: &mut Box<dyn Session>, kind: ObjectKind) -> Vec<String> {
    MysqlDriver
        .introspector()
        .objects(s.as_mut(), "app", None, kind)
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
    let (_db, mut s) = setup().await;
    let i = MysqlDriver.introspector();
    let dbs = i.databases(s.as_mut()).await.unwrap();

    // Act & Assert
    for want in ["app", "information_schema"] {
        assert!(dbs.contains(&want.into()), "{want}: {dbs:?}");
    }
    assert!(i.schemas(s.as_mut(), "app").await.unwrap().is_empty());

    use ObjectKind::*;
    let cases: [(ObjectKind, &[&str]); 5] = [
        (Table, &["customers", "orders"]),
        (View, &["order_view"]),
        (Function, &["add_one"]),
        (Procedure, &["noop"]),
        (Trigger, &["orders_touch"]),
    ];
    for (kind, want) in cases {
        let got = names(&mut s, kind).await;
        for w in want {
            assert!(got.contains(&(*w).into()), "{kind:?} {w}: {got:?}");
        }
    }
    assert!(!names(&mut s, Table).await.contains(&"order_view".into()));
    assert!(!names(&mut s, View).await.contains(&"orders".into()));
    let t = i.objects(s.as_mut(), "app", None, Table).await.unwrap();
    assert_eq!(t[0], app("customers", Table));
}

#[tokio::test]
#[ignore = "docker"]
async fn row_estimate_without_count() {
    // Arrange
    let (_db, mut s) = setup().await;
    run(
        &mut s,
        "CREATE TABLE app.est_a (g INT PRIMARY KEY) ENGINE=InnoDB",
    )
    .await;
    run(
        &mut s,
        "INSERT INTO app.est_a SELECT a.d + 10 * b.d + 100 * c.d FROM app.digits a, app.digits b, app.digits c",
    )
    .await;
    run(&mut s, "ANALYZE TABLE app.est_a").await;

    // Act & Assert
    let i = MysqlDriver.introspector();
    let a = i
        .row_estimate(s.as_mut(), &app("est_a", ObjectKind::Table))
        .await
        .unwrap()
        .unwrap();
    assert!((900..=1100).contains(&a), "{a}");
    let v = i
        .row_estimate(s.as_mut(), &app("order_view", ObjectKind::View))
        .await
        .unwrap();
    assert_eq!(v, None);
}

#[tokio::test]
#[ignore = "docker"]
async fn ddl_for_each_object_kind() {
    // Arrange
    let (_db, mut s) = setup().await;
    use ObjectKind::*;
    let all = [
        app("customers", Table),
        app("orders", Table),
        app("order_view", View),
        app("add_one", Function),
        app("noop", Procedure),
        app("orders_touch", Trigger),
    ];
    let i = MysqlDriver.introspector();

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
}

#[tokio::test]
#[ignore = "docker"]
async fn structure_lists_columns_indexes_fks() {
    // Arrange
    let (_db, mut s) = setup().await;

    // Act & Assert
    let i = MysqlDriver.introspector();
    let cols = i
        .columns(s.as_mut(), &app("customers", ObjectKind::Table))
        .await
        .unwrap();
    let n: Vec<_> = cols.iter().map(|c| c.meta.name.as_str()).collect();
    assert_eq!(n, ["id", "name", "email", "ci", "amt", "pic"]);
    assert!(cols[0].meta.is_primary_key && !cols[1].meta.is_primary_key);
    assert!(cols[0].meta.type_name.starts_with("int"));
    assert_eq!(cols[0].meta.category, TypeCategory::Integer);
    assert_eq!(cols[0].meta.nullable, Some(false));
    assert_eq!(cols[2].meta.nullable, Some(true));
    assert_eq!(cols[1].comment.as_deref(), Some("full name"));
    assert_eq!(cols[1].meta.type_name, "varchar(100)");
    assert_eq!(cols[1].meta.category, TypeCategory::Text);
    assert_eq!(cols[4].meta.category, TypeCategory::Decimal);
    assert_eq!(cols[5].meta.category, TypeCategory::Binary);

    let orders = app("orders", ObjectKind::Table);
    let ocols = i.columns(s.as_mut(), &orders).await.unwrap();
    assert_eq!(ocols[2].default.as_deref(), Some("1"));
    let idx = i.indexes(s.as_mut(), &orders).await.unwrap();
    let pk = idx.iter().find(|x| x.name == "PRIMARY").unwrap();
    assert!(pk.primary && pk.unique && pk.columns == ["id"]);
    let ci = idx
        .iter()
        .find(|x| x.name == "orders_customer_idx")
        .unwrap();
    assert!(!ci.unique && !ci.primary);
    assert_eq!(ci.columns, ["customer_id", "qty"]);
    assert_eq!(ci.method.as_deref(), Some("BTREE"));

    let fks = i.foreign_keys(s.as_mut(), &orders).await.unwrap();
    assert_eq!(fks.len(), 1);
    let fk = &fks[0];
    assert_eq!(fk.columns, ["customer_id"]);
    assert_eq!(fk.ref_table, app("customers", ObjectKind::Table));
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
    let columns = MysqlDriver
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
    let b = build_table_query(&MysqlDialect, &q).unwrap();
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
        "CREATE TABLE app.grid (id INT, name VARCHAR(20), score INT, note VARCHAR(20), tag CHAR(1))",
    )
    .await;
    run(
        s,
        "INSERT INTO app.grid SELECT g, CONCAT('name', g), g % 5,
            CASE WHEN g % 6 = 0 THEN NULL WHEN g % 2 = 0 THEN '50% off' ELSE 'plain' END,
            ELT(g % 3 + 1, 'A', 'B', 'C')
         FROM (SELECT a.d + 10 * b.d + 1 AS g FROM app.digits a, app.digits b) x WHERE g <= 30",
    )
    .await;
    app("grid", ObjectKind::Table)
}

fn ids(p: &Page) -> Vec<u32> {
    p.iter()
        .take(10)
        .map(|r| r[0].parse::<u32>().unwrap())
        .collect()
}

#[tokio::test]
#[ignore = "docker"]
async fn fetches_page_with_filters_and_sorts() {
    // Arrange
    let (_db, mut s) = setup().await;
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
    let cases = [
        (f("note", Operator::Contains, &["%"]), 10),
        (f("tag", Operator::In, &["A", "B"]), 20),
        (f("id", Operator::Between, &["5", "9"]), 5),
        (f("note", Operator::IsNull, &[]), 5),
    ];
    for (flt, want) in cases {
        let p = page(&mut s, &g, vec![flt.clone()], vec![], 0, 100).await;
        assert_eq!(p.len(), want, "{flt:?}");
    }

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
    let (_db, mut s) = setup().await;
    let g = grid_table(&mut s).await;
    run(&mut s, "ALTER TABLE app.grid ADD PRIMARY KEY (id)").await;
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
    let (_db, mut s) = setup().await;
    let v = s.server_version();
    // JSON arrived in MySQL 5.7; MariaDB aliases it to LONGTEXT since 10.2.
    let floor = if v.product == Product::Mysql {
        (5, 7)
    } else {
        (10, 2)
    };
    if (v.major, v.minor) < floor {
        eprintln!("skipped: {} has no JSON type", v.raw);
        return;
    }
    run(&mut s, "CREATE TABLE app.j (id INT PRIMARY KEY, doc JSON)").await;
    run(
        &mut s,
        "INSERT INTO app.j VALUES (1, '{\"b\":1}'), (2, '{\"a\":1}')",
    )
    .await;
    let sorts = vec![Sort {
        column: "doc".into(),
        descending: false,
    }];

    // Act
    let rows = page(&mut s, &app("j", ObjectKind::Table), vec![], sorts, 0, 10).await;

    // Assert
    assert_eq!(rows[0][0], "2");
}

#[tokio::test]
#[ignore = "docker"]
async fn filter_value_injection_is_inert() {
    // Arrange
    let (_db, mut s) = setup().await;
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
async fn contains_is_case_insensitive_under_binary_collation() {
    // Arrange
    let (_db, mut s) = setup().await;
    run(
        &mut s,
        "INSERT INTO app.customers (name, ci) VALUES ('n', 'Hello World')",
    )
    .await;
    let c = app("customers", ObjectKind::Table);

    // Act
    let rows = page(
        &mut s,
        &c,
        vec![f("ci", Operator::Contains, &["hello"])],
        vec![],
        0,
        10,
    )
    .await;

    // Assert
    assert_eq!(rows.len(), 1, "{rows:?}");
}

#[tokio::test]
#[ignore = "docker"]
async fn table_with_quoted_mixed_case_name_opens() {
    // Arrange
    let (_db, mut s) = setup().await;
    run(&mut s, "CREATE DATABASE `Mi``xed Db`").await;
    run(
        &mut s,
        "CREATE TABLE `Mi``xed Db`.`we``ird` (`Mixed Col` INT, `Val` VARCHAR(10))",
    )
    .await;
    run(
        &mut s,
        "INSERT INTO `Mi``xed Db`.`we``ird` VALUES (1, 'x'), (2, 'y')",
    )
    .await;

    // Act & Assert
    let i = MysqlDriver.introspector();
    let tables = i
        .objects(s.as_mut(), "Mi`xed Db", None, ObjectKind::Table)
        .await
        .unwrap();
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].name, "we`ird");
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
    assert!(ddl.contains("`we``ird`"), "{ddl}");
}
