#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use crustdb_app::runtime::{
    Command, DriverRegistry, Event, ResolvedSecrets, Runtime, TabId, WorkspaceId,
};
use crustdb_app::storage::config::ConnectionConfig;
use crustdb_core::driver::{ConnectTarget, Driver, ExecMode, SessionPurpose, TlsConfig, TlsMode};
use crustdb_core::model::{
    ColumnMeta, ConnectionId, DriverKind, ObjectKind, ObjectRef, TypeCategory,
};
use crustdb_core::query::TableQuery;
use secrecy::SecretString;

const ROWS: u64 = 20_000;

fn col(name: &str, category: TypeCategory, type_name: &str) -> ColumnMeta {
    ColumnMeta {
        name: name.into(),
        type_name: type_name.into(),
        category,
        nullable: Some(false),
        is_primary_key: name == "id",
    }
}

async fn export_table(
    driver: Arc<dyn Driver>,
    kind: DriverKind,
    (host, port, user, password): (String, u16, String, String),
    (database, schema, seed): (&str, Option<&str>, &[String]),
) {
    let tls = TlsConfig {
        mode: TlsMode::Disable,
        ca_path: None,
        client_cert_path: None,
        client_key_path: None,
    };
    let target = ConnectTarget {
        host: host.clone(),
        port,
        connect_addr: None,
        user: user.clone(),
        password: Some(SecretString::from(password.clone())),
        database: None,
        tls: tls.clone(),
        connect_timeout: Duration::from_secs(10),
    };
    let mut s = driver
        .connect(&target, SessionPurpose::Editor)
        .await
        .unwrap();
    for sql in seed {
        s.start(sql, ExecMode::Other).await.unwrap();
        s.finish().await.unwrap();
    }
    s.close().await;

    let (rt, rx) = Runtime::start(
        Arc::new(|| {}),
        DriverRegistry::default().with(driver.clone()),
    );
    let (ws, tab) = (WorkspaceId(1), TabId(1));
    rt.send(Command::Connect {
        ws,
        config: ConnectionConfig {
            id: ConnectionId(uuid::Uuid::nil()),
            name: "it".into(),
            driver: kind,
            host,
            port,
            user,
            database: Some(database.into()),
            save_password: false,
            tls,
            ssh: None,
            page_size: 300,
            editor_row_limit: 1000,
            safety_threshold: 1000,
            label: None,
            key_separator: None,
        },
        secrets: ResolvedSecrets {
            db: Some(SecretString::from(password)),
            ..ResolvedSecrets::default()
        },
    });
    let object = ObjectRef {
        database: database.into(),
        schema: schema.map(String::from),
        name: "export_t".into(),
        kind: ObjectKind::Table,
    };
    loop {
        match rx.recv_timeout(Duration::from_secs(60)).unwrap() {
            Event::Connected { .. } => break,
            Event::ConnectFailed { error, .. } => panic!("{error}"),
            _ => {}
        }
    }
    rt.send(Command::OpenTable {
        ws,
        tab,
        object: object.clone(),
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.csv");
    let req = rt.next_request_id();
    rt.send(Command::ExportTable {
        tab,
        req,
        query: TableQuery {
            object,
            columns: vec![
                col("id", TypeCategory::Integer, "int"),
                col("v", TypeCategory::Text, "text"),
            ],
            filters: vec![],
            sorts: vec![],
            limit: Some(300),
            offset: 0,
            after: None,
        },
        path: path.clone(),
    });
    let mut progress = 0;
    let result = loop {
        match rx.recv_timeout(Duration::from_secs(120)).unwrap() {
            Event::ExportProgress { .. } => progress += 1,
            Event::ExportDone { result, .. } => break result,
            _ => {}
        }
    };
    assert_eq!(result, Ok(ROWS));
    assert!(progress >= 4);
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().count() as u64, ROWS + 1);
    assert!(text.starts_with("id,v\n"));
    rt.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "docker"]
async fn export_full_table_streams_csv_postgres() {
    // Arrange
    let pg = crustdb_testkit::start_postgres(None, None).await;
    let seed = [
        "CREATE TABLE public.export_t (id int PRIMARY KEY, v text)".to_string(),
        format!("INSERT INTO public.export_t SELECT g, 'v' || g FROM generate_series(1, {ROWS}) g"),
    ];

    // Act & Assert
    export_table(
        Arc::new(crustdb_postgres::PostgresDriver),
        DriverKind::Postgres,
        (
            pg.host.clone(),
            pg.port,
            pg.user.clone(),
            pg.password.clone(),
        ),
        ("postgres", Some("public"), &seed),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "docker"]
async fn export_full_table_streams_csv_mysql() {
    // Arrange
    let my = crustdb_testkit::start_mysql(None, None).await;
    let seed = [
        "CREATE DATABASE exp".to_string(),
        "CREATE TABLE exp.export_t (id int PRIMARY KEY, v text)".to_string(),
        "CREATE TABLE exp.d (i int)".to_string(),
        "INSERT INTO exp.d VALUES (0),(1),(2),(3),(4),(5),(6),(7),(8),(9)".to_string(),
        // Cross join instead of a recursive CTE: MySQL 5.6/5.7 and MariaDB 10.0 lack CTEs.
        format!(
            "INSERT INTO exp.export_t SELECT n, CONCAT('v', n) FROM (SELECT 1 + a.i + 10 * b.i + 100 * c.i + 1000 * d.i + 10000 * e.i AS n FROM exp.d a, exp.d b, exp.d c, exp.d d, exp.d e) s WHERE n <= {ROWS}"
        ),
    ];

    // Act & Assert
    export_table(
        Arc::new(crustdb_mysql::MysqlDriver),
        DriverKind::Mysql,
        (
            my.host.clone(),
            my.port,
            my.user.clone(),
            my.password.clone(),
        ),
        ("exp", None, &seed),
    )
    .await;
}
