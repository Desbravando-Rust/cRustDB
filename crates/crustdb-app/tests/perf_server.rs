#![allow(clippy::unwrap_used, clippy::expect_used, clippy::err_expect)]
//! Spec §7 budgets that need a live server (Docker). Release build, one thread:
//! `cargo test --release -p crustdb-app --test perf_server -- --include-ignored --test-threads=1 --nocapture`
//! Measures, per driver: (a) time from the server's first row to the first `Event::Rows` and
//! (b) a 100,000 x 20 CSV export end to end (rows/s and peak RSS growth).
use std::sync::Arc;
use std::time::{Duration, Instant};

use crustdb_app::runtime::{
    Command, DriverRegistry, Event, ResolvedSecrets, Runtime, TabId, WorkspaceId,
};
use crustdb_app::storage::config::ConnectionConfig;
use crustdb_core::driver::{
    ConnectTarget, Driver, ExecMode, Session, SessionPurpose, TlsConfig, TlsMode,
};
use crustdb_core::model::{
    ColumnMeta, ConnectionId, DriverKind, ObjectKind, ObjectRef, TypeCategory,
};
use crustdb_core::query::TableQuery;
use secrecy::SecretString;

const ROWS: u64 = 100_000;
const COLS: usize = 20;
const RUNS: usize = 5;

struct Server {
    driver: Arc<dyn Driver>,
    kind: DriverKind,
    host: String,
    port: u16,
    user: String,
    password: String,
    database: &'static str,
    schema: Option<&'static str>,
    seed: Vec<String>,
    table: &'static str,
}

fn tls() -> TlsConfig {
    TlsConfig {
        mode: TlsMode::Disable,
        ca_path: None,
        client_cert_path: None,
        client_key_path: None,
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    v[v.len() / 2]
}

/// Reads one `Vm*:` line of `/proc/self/status`, in MB.
fn vm_mb(key: &str) -> f64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap();
    let line = s.lines().find(|l| l.starts_with(key)).unwrap();
    let kb: f64 = line.split_whitespace().nth(1).unwrap().parse().unwrap();
    kb / 1024.0
}

fn col(name: &str) -> ColumnMeta {
    ColumnMeta {
        name: name.into(),
        type_name: "text".into(),
        category: TypeCategory::Text,
        nullable: Some(false),
        is_primary_key: name == "id",
    }
}

fn config(s: &Server) -> ConnectionConfig {
    ConnectionConfig {
        id: ConnectionId(uuid::Uuid::nil()),
        name: "perf".into(),
        driver: s.kind,
        host: s.host.clone(),
        port: s.port,
        user: s.user.clone(),
        database: Some(s.database.into()),
        save_password: false,
        tls: tls(),
        ssh: None,
        page_size: 300,
        editor_row_limit: 1000,
        safety_threshold: 1000,
        label: None,
        key_separator: None,
    }
}

async fn session(s: &Server) -> Box<dyn Session> {
    let target = ConnectTarget {
        host: s.host.clone(),
        port: s.port,
        connect_addr: None,
        user: s.user.clone(),
        password: Some(SecretString::from(s.password.clone())),
        database: None,
        tls: tls(),
        connect_timeout: Duration::from_secs(10),
    };
    s.driver
        .connect(&target, SessionPurpose::Editor)
        .await
        .unwrap()
}

async fn seed(s: &Server) {
    let mut c = session(s).await;
    for sql in &s.seed {
        c.start(sql, ExecMode::Other).await.unwrap();
        c.finish().await.unwrap();
    }
    c.close().await;
}

fn connect(s: &Server) -> (Runtime, std::sync::mpsc::Receiver<Event>) {
    let (rt, rx) = Runtime::start(
        Arc::new(|| {}),
        DriverRegistry::default().with(s.driver.clone()),
    );
    rt.send(Command::Connect {
        ws: WorkspaceId(1),
        config: config(s),
        secrets: ResolvedSecrets {
            db: Some(SecretString::from(s.password.clone())),
            ..ResolvedSecrets::default()
        },
    });
    loop {
        match rx.recv_timeout(Duration::from_secs(60)).unwrap() {
            Event::Connected { .. } => return (rt, rx),
            Event::ConnectFailed { error, .. } => panic!("{error}"),
            _ => {}
        }
    }
}

/// Median ms from the driver's first row (`start` + `fetch(1)` returned) to the first `Event::Rows`
/// of the same `SELECT` run through the editor actor (which fetches the whole 1,000-row limit first).
async fn first_rows_after_first_row(s: &Server) -> (f64, f64, f64) {
    let sql = format!("SELECT * FROM {}", qualified(s));
    let mut c = session(s).await;
    let (rt, rx) = connect(s);
    let (ws, tab) = (WorkspaceId(1), TabId(1));
    rt.send(Command::OpenEditor {
        ws,
        tab,
        database: None,
    });
    let (mut first_row, mut to_rows) = (vec![], vec![]);
    for run in 0..=RUNS {
        // Arrange
        let t = Instant::now();
        c.start(&sql, ExecMode::Read).await.unwrap();
        c.fetch(1).await.unwrap();
        let first = t.elapsed();
        c.finish().await.unwrap();
        let req = rt.next_request_id();
        rt.send(Command::Evaluate {
            tab,
            req,
            statements: vec![sql.clone()],
        });
        loop {
            match rx.recv_timeout(Duration::from_secs(60)).unwrap() {
                Event::Evaluated { .. } => break,
                Event::EvaluateFailed { text, .. } => panic!("{text}"),
                _ => {}
            }
        }

        // Act
        let t = Instant::now();
        rt.send(Command::Execute {
            tab,
            req: rt.next_request_id(),
            evaluation: req,
        });
        let rows_at = loop {
            match rx.recv_timeout(Duration::from_secs(60)).unwrap() {
                Event::Rows { .. } => break t.elapsed(),
                Event::ExecError { error, .. } => panic!("{error}"),
                _ => {}
            }
        };

        // Assert: run 0 only warms caches.
        if run > 0 {
            first_row.push(ms(first));
            to_rows.push(ms(rows_at));
        }
    }
    rt.send(Command::CloseTab { tab, resolve: None });
    rt.shutdown();
    c.close().await;
    let (f, r) = (median(first_row), median(to_rows));
    (f, r, r - f)
}

fn qualified(s: &Server) -> String {
    match s.schema {
        Some(sc) => format!("{sc}.{}", s.table),
        None => format!("{}.{}", s.database, s.table),
    }
}

/// Median of `RUNS` exports: (rows/s, peak RSS growth in MB).
fn export_100k(s: &Server) -> (f64, f64) {
    let (rt, rx) = connect(s);
    let (ws, tab) = (WorkspaceId(1), TabId(1));
    let object = ObjectRef {
        database: s.database.into(),
        schema: s.schema.map(String::from),
        name: s.table.into(),
        kind: ObjectKind::Table,
    };
    rt.send(Command::OpenTable {
        ws,
        tab,
        object: object.clone(),
    });
    let mut columns = vec![col("id")];
    columns.extend((0..COLS).map(|c| col(&format!("c{c}"))));
    let dir = tempfile::tempdir().unwrap();
    let (mut rates, mut grew) = (vec![], vec![]);
    for i in 0..3 {
        // Arrange
        let path = dir.path().join(format!("out{i}.csv"));
        // Resets VmHWM to the current RSS.
        std::fs::write("/proc/self/clear_refs", "5").unwrap();
        let before = vm_mb("VmRSS:");
        let req = rt.next_request_id();

        // Act
        let t = Instant::now();
        rt.send(Command::ExportTable {
            tab,
            req,
            query: TableQuery {
                object: object.clone(),
                columns: columns.clone(),
                filters: vec![],
                sorts: vec![],
                limit: Some(300),
                offset: 0,
                after: None,
            },
            path: path.clone(),
        });
        let result = loop {
            if let Event::ExportDone { result, .. } =
                rx.recv_timeout(Duration::from_secs(300)).unwrap()
            {
                break result;
            }
        };
        let took = t.elapsed();

        // Assert
        assert_eq!(result, Ok(ROWS));
        rates.push(ROWS as f64 / took.as_secs_f64());
        grew.push(vm_mb("VmHWM:") - before);
        std::fs::remove_file(path).unwrap();
    }
    rt.shutdown();
    (median(rates), median(grew))
}

async fn run(s: Server, label: &str) {
    if cfg!(debug_assertions) {
        return;
    }
    // Arrange
    seed(&s).await;

    // Act
    let (first, rows_at, delta) = first_rows_after_first_row(&s).await;
    let (rate, grew) = export_100k(&s);

    // Assert
    println!(
        "PERF {label} first row {first:.1} ms, first Rows {rows_at:.1} ms, after first row {delta:.1} ms"
    );
    println!("PERF {label} export 100k x {COLS}: {rate:.0} rows/s, peak RSS +{grew:.1} MB");
    assert!(delta <= 50.0, "{delta:.1} ms after the first row");
    assert!(rate >= 100_000.0, "{rate:.0} rows/s");
    assert!(grew <= 20.0, "+{grew:.1} MB");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "perf"]
async fn postgres_first_rows_and_export() {
    let pg = crustdb_testkit::start_postgres(None, None).await;
    let cols: String = (0..COLS)
        .map(|c| format!(", 'r' || lpad(g::text, 7, '0') || '-c{c:03}-xxxxx'"))
        .collect();
    let defs: String = (0..COLS).map(|c| format!(", c{c} text")).collect();
    let seed = vec![
        format!("CREATE TABLE public.perf_t (id int PRIMARY KEY{defs})"),
        format!("INSERT INTO public.perf_t SELECT g{cols} FROM generate_series(1, {ROWS}) g"),
    ];

    run(
        Server {
            driver: Arc::new(crustdb_postgres::PostgresDriver),
            kind: DriverKind::Postgres,
            host: pg.host.clone(),
            port: pg.port,
            user: pg.user.clone(),
            password: pg.password.clone(),
            database: "postgres",
            schema: Some("public"),
            seed,
            table: "perf_t",
        },
        "postgres",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "perf"]
async fn mysql_first_rows_and_export() {
    let my = crustdb_testkit::start_mysql(None, None).await;
    let cols: String = (0..COLS)
        .map(|c| format!(", CONCAT('r', LPAD(n, 7, '0'), '-c{c:03}-xxxxx')"))
        .collect();
    let defs: String = (0..COLS).map(|c| format!(", c{c} varchar(32)")).collect();
    let seed = vec![
        "CREATE DATABASE perf".to_string(),
        format!("CREATE TABLE perf.perf_t (id int PRIMARY KEY{defs})"),
        "CREATE TABLE perf.d (i int)".to_string(),
        "INSERT INTO perf.d VALUES (0),(1),(2),(3),(4),(5),(6),(7),(8),(9)".to_string(),
        format!(
            "INSERT INTO perf.perf_t SELECT n{cols} FROM (SELECT 1 + a.i + 10 * b.i + 100 * c.i + 1000 * d.i + 10000 * e.i AS n FROM perf.d a, perf.d b, perf.d c, perf.d d, perf.d e) s WHERE n <= {ROWS}"
        ),
    ];

    run(
        Server {
            driver: Arc::new(crustdb_mysql::MysqlDriver),
            kind: DriverKind::Mysql,
            host: my.host.clone(),
            port: my.port,
            user: my.user.clone(),
            password: my.password.clone(),
            database: "perf",
            schema: None,
            seed,
            table: "perf_t",
        },
        "mysql",
    )
    .await;
}
