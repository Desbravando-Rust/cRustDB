#![allow(clippy::unwrap_used, clippy::expect_used, clippy::err_expect)] // Box<dyn Session> is not Debug

use std::time::{Duration, Instant};

use crustdb_core::driver::{
    ConnectTarget, Driver, ExecMode, Param, Session, SessionPurpose, TlsConfig, TlsMode,
};
use crustdb_core::error::ErrorKind;
use crustdb_core::model::Cell;
use crustdb_mysql::MysqlDriver;
use crustdb_testkit::{MysqlContainer, start_mysql};
use secrecy::SecretString;

async fn editor(db: &MysqlContainer) -> Box<dyn Session> {
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

/// 10^7 rows without a stored table: cross join of ten digits.
const TEN_MILLION: &str =
    "SELECT a.n FROM digits a, digits b, digits c, digits d, digits e, digits f, digits g";

async fn with_digits(s: &mut Box<dyn Session>) {
    all(s, "CREATE TABLE digits(n int)", ExecMode::Other).await;
    all(
        s,
        "INSERT INTO digits VALUES (0),(1),(2),(3),(4),(5),(6),(7),(8),(9)",
        ExecMode::Other,
    )
    .await;
}

async fn use_test_db(s: &mut Box<dyn Session>) {
    all(s, "CREATE DATABASE IF NOT EXISTS t", ExecMode::Other).await;
    s.use_database("t").await.unwrap();
}

#[tokio::test]
#[ignore = "docker"]
async fn read_cursor_load_more_bounded_memory() {
    let db = start_mysql(None, None).await;
    let mut s = editor(&db).await;
    use_test_db(&mut s).await;
    with_digits(&mut s).await;
    let before = rss_kb();
    s.start(TEN_MILLION, ExecMode::Read).await.unwrap();
    let a = s.fetch(1000).await.unwrap();
    let b = s.fetch(1000).await.unwrap();
    assert!(!a.exhausted && !b.exhausted);
    assert_eq!((a.rows.len(), b.rows.len()), (1000, 1000));
    assert!(s.has_active_result());
    let t = Instant::now();
    s.finish().await.unwrap();
    assert!(t.elapsed() < Duration::from_secs(10));
    assert!(!s.has_active_result());
    assert!(rss_kb().saturating_sub(before) < 50 * 1024);
    assert_eq!(all(&mut s, "SELECT 1", ExecMode::Read).await[0][0], "1");
}

#[tokio::test]
#[ignore = "docker"]
async fn zero_row_select_keeps_columns() {
    let db = start_mysql(None, None).await;
    let mut s = editor(&db).await;
    for mode in [ExecMode::Read, ExecMode::Other] {
        let h = s
            .start("SELECT 1 AS a FROM DUAL WHERE false", mode)
            .await
            .unwrap();
        assert_eq!(h.columns.len(), 1);
        assert_eq!(h.columns[0].name, "a");
        let f = s.fetch(10).await.unwrap();
        assert!(f.rows.is_empty() && f.exhausted);
        s.finish().await.unwrap();
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn write_is_drained_not_cancelled() {
    let db = start_mysql(None, None).await;
    let mut s = editor(&db).await;
    use_test_db(&mut s).await;
    with_digits(&mut s).await;
    all(&mut s, "CREATE TABLE w(id int)", ExecMode::Other).await;
    let h = s
        .start(
            "INSERT INTO w SELECT a.n FROM digits a, digits b, digits c",
            ExecMode::Other,
        )
        .await
        .unwrap();
    assert!(h.columns.is_empty());
    let c = s.finish().await.unwrap();
    assert_eq!(c.affected_rows, Some(1000));
    assert_eq!(
        all(&mut s, "SELECT count(*) FROM w", ExecMode::Read).await[0][0],
        "1000"
    );
    // Rows are fetched partially, the rest is drained to the end (D29).
    s.start(
        "SELECT a.n FROM digits a, digits b, digits c",
        ExecMode::Other,
    )
    .await
    .unwrap();
    assert_eq!(s.fetch(10).await.unwrap().rows.len(), 10);
    s.finish().await.unwrap();
    assert_eq!(all(&mut s, "SELECT 1", ExecMode::Read).await[0][0], "1");
}

#[tokio::test]
#[ignore = "docker"]
async fn paused_cursor_finished_before_next_command() {
    let db = start_mysql(None, None).await;
    let mut s = editor(&db).await;
    let mut other = editor(&db).await;
    use_test_db(&mut s).await;
    with_digits(&mut s).await;
    let cid = all(&mut s, "SELECT CONNECTION_ID()", ExecMode::Read).await[0][0].clone();
    s.start(TEN_MILLION, ExecMode::Read).await.unwrap();
    s.fetch(10).await.unwrap();
    assert!(s.has_active_result());
    s.start("SELECT 2", ExecMode::Other).await.unwrap();
    assert_eq!(txt(&s.fetch(10).await.unwrap().rows[0][0]), "2");
    s.finish().await.unwrap();
    let q = format!("SELECT command FROM information_schema.processlist WHERE id = {cid}");
    assert_eq!(all(&mut other, &q, ExecMode::Read).await[0][0], "Sleep");
    // A paused cursor is also finished by non-start commands.
    s.start(TEN_MILLION, ExecMode::Read).await.unwrap();
    s.fetch(10).await.unwrap();
    s.explain_rows("SELECT 1").await.unwrap();
    assert!(!s.has_active_result());
}

#[tokio::test]
#[ignore = "docker"]
async fn cancel_long_query() {
    let db = start_mysql(None, None).await;
    let mut s = editor(&db).await;
    let h = s.cancel_handle();
    let t = Instant::now();
    let q = tokio::spawn(async move {
        // A killed SLEEP() returns normally (value 1) instead of erroring.
        let r = async {
            s.start("SELECT SLEEP(60)", ExecMode::Read).await?;
            s.fetch(10).await
        }
        .await;
        (s, r)
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    h.cancel().await.unwrap();
    let (mut s, r) = q.await.unwrap();
    // MySQL returns normally, MariaDB reports 1317.
    if let Err(e) = r {
        assert_eq!(e.kind, ErrorKind::Cancelled);
    }
    assert!(
        t.elapsed() < Duration::from_millis(2500),
        "{:?}",
        t.elapsed()
    );
    s.finish().await.unwrap();
    assert_eq!(all(&mut s, "SELECT 1", ExecMode::Read).await[0][0], "1");

    // A heavy scan does surface the interruption (1317).
    let h = s.cancel_handle();
    let q = tokio::spawn(async move {
        let r = async {
            s.start(
                "SELECT COUNT(*) FROM information_schema.columns a, information_schema.columns b, information_schema.columns c",
                ExecMode::Read,
            )
            .await?;
            s.fetch(10).await
        }
        .await;
        (s, r)
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    h.cancel().await.unwrap();
    let (mut s, r) = q.await.unwrap();
    assert_eq!(r.err().unwrap().kind, ErrorKind::Cancelled);
    s.finish().await.unwrap();
    assert_eq!(all(&mut s, "SELECT 1", ExecMode::Read).await[0][0], "1");
}

#[tokio::test]
#[ignore = "docker"]
async fn explain_rows_for_update_delete() {
    let db = start_mysql(None, None).await;
    let mut s = editor(&db).await;
    use_test_db(&mut s).await;
    with_digits(&mut s).await;
    all(
        &mut s,
        "CREATE TABLE t(id int PRIMARY KEY, a int)",
        ExecMode::Other,
    )
    .await;
    all(
        &mut s,
        "INSERT INTO t SELECT a.n + 10*b.n + 100*c.n + 1000*d.n, 0 FROM digits a, digits b, digits c, digits d",
        ExecMode::Other,
    )
    .await;
    all(&mut s, "ANALYZE TABLE t", ExecMode::Other).await;
    for sql in [
        "UPDATE t SET a = 1 WHERE id <= 5000",
        "DELETE FROM t WHERE id <= 5000",
    ] {
        let n = s.explain_rows(sql).await.unwrap().unwrap();
        assert!((2500..=10000).contains(&n), "{sql}: {n}");
    }
    // The target table wins over the other (tiny) table of a join.
    let n = s
        .explain_rows("UPDATE t JOIN digits ON digits.n = t.a SET t.a = 2")
        .await
        .unwrap()
        .unwrap();
    assert!(n >= 5000, "{n}");
    assert!(s.explain_rows("SELECT 1").await.unwrap().is_none());
    assert!(s.explain_rows("SELEKT nope").await.is_err());
}

#[tokio::test]
#[ignore = "docker"]
async fn transaction_states_and_implicit_commit_mysql() {
    let db = start_mysql(None, None).await;
    let mut s = editor(&db).await;
    use_test_db(&mut s).await;
    all(&mut s, "CREATE TABLE tx(a int)", ExecMode::Other).await;
    for sql in [
        "BEGIN",
        "INSERT INTO tx VALUES (1)",
        "CREATE TABLE tx2(a int)", // implicit commit
        "ROLLBACK",
    ] {
        all(&mut s, sql, ExecMode::Other).await;
    }
    assert_eq!(
        all(&mut s, "SELECT count(*) FROM tx", ExecMode::Read).await[0][0],
        "1"
    );
}

#[tokio::test]
#[ignore = "docker"]
async fn prepared_datetime_formats() {
    let db = start_mysql(None, None).await;
    let mut s = editor(&db).await;
    use_test_db(&mut s).await;
    all(
        &mut s,
        "CREATE TABLE dt(d DATE, dt DATETIME(6), ts TIMESTAMP NULL, t TIME, i BIGINT, f DOUBLE, n DECIMAL(5,2), x INT)",
        ExecMode::Other,
    )
    .await;
    all(
        &mut s,
        "INSERT INTO dt VALUES ('2024-01-02', '2024-01-02 03:04:05.5', '2024-01-02 03:04:05', '-838:59:59', -7, 1.5, 3.10, NULL)",
        ExecMode::Other,
    )
    .await;
    let p = |v: &str| Param(Some(v.into()));
    let h = s
        .start_params(
            "SELECT d, dt, ts, t, i, f, n, x FROM dt WHERE i = ?",
            &[p("-7")],
        )
        .await
        .unwrap();
    assert_eq!(
        h.columns[0].category,
        crustdb_core::model::TypeCategory::Date
    );
    let f = s.fetch(10).await.unwrap();
    s.finish().await.unwrap();
    let got: Vec<_> = f.rows[0]
        .iter()
        .map(|c| match c {
            Cell::Null => "NULL".to_string(),
            c => txt(c).to_string(),
        })
        .collect();
    assert_eq!(
        got,
        [
            "2024-01-02",
            "2024-01-02 03:04:05.500000",
            "2024-01-02 03:04:05",
            "-838:59:59",
            "-7",
            "1.5",
            "3.10",
            "NULL"
        ]
    );
    // NULL parameter.
    s.start_params("SELECT ? IS NULL", &[Param(None)])
        .await
        .unwrap();
    assert_eq!(txt(&s.fetch(1).await.unwrap().rows[0][0]), "1");
    s.finish().await.unwrap();
}

#[tokio::test]
#[ignore = "docker"]
async fn latin1_column_displays() {
    let db = start_mysql(None, None).await;
    let mut s = editor(&db).await;
    use_test_db(&mut s).await;
    all(
        &mut s,
        "CREATE TABLE l(c VARCHAR(10) CHARACTER SET latin1, b VARBINARY(4))",
        ExecMode::Other,
    )
    .await;
    all(
        &mut s,
        "INSERT INTO l VALUES (UNHEX('E9'), UNHEX('00FF'))",
        ExecMode::Other,
    )
    .await;
    s.start("SELECT c, b FROM l", ExecMode::Read).await.unwrap();
    let f = s.fetch(10).await.unwrap();
    s.finish().await.unwrap();
    assert_eq!(txt(&f.rows[0][0]), "é");
    assert!(
        matches!(&f.rows[0][1], Cell::Bytes { value, total_len: 2, .. } if **value == [0, 0xFF])
    );
}

#[tokio::test]
#[ignore = "docker"]
async fn use_database_switches_and_quotes() {
    let db = start_mysql(None, None).await;
    let mut s = editor(&db).await;
    all(&mut s, "CREATE DATABASE `we``ird`", ExecMode::Other).await;
    s.use_database("we`ird").await.unwrap();
    assert_eq!(
        all(&mut s, "SELECT DATABASE()", ExecMode::Read).await[0][0],
        "we`ird"
    );
    assert!(s.use_database("missing").await.is_err());
}

#[tokio::test]
#[ignore = "docker"]
async fn server_restart_during_paused_cursor_reports_error() {
    let db = start_mysql(None, None).await;
    let mut s = editor(&db).await;
    use_test_db(&mut s).await;
    with_digits(&mut s).await;
    s.start(TEN_MILLION, ExecMode::Read).await.unwrap();
    s.fetch(10).await.unwrap();
    db.restart().await;
    // Rows already buffered in the socket may still arrive; the stream must end in an error.
    let mut err = None;
    for _ in 0..200 {
        match s.fetch(100_000).await {
            Ok(f) if f.exhausted => break,
            Ok(_) => {}
            Err(e) => {
                err = Some(e);
                break;
            }
        }
    }
    assert_eq!(err.expect("error expected").kind, ErrorKind::Connect);
    let _ = s.finish().await;
    assert!(!s.has_active_result());
}

#[tokio::test]
#[ignore = "docker"]
async fn dropped_session_kills_paused_read() {
    let db = start_mysql(None, None).await;
    let mut s = editor(&db).await;
    let mut other = editor(&db).await;
    use_test_db(&mut s).await;
    with_digits(&mut s).await;
    let cid = all(&mut s, "SELECT CONNECTION_ID()", ExecMode::Read).await[0][0].clone();
    s.start(TEN_MILLION, ExecMode::Read).await.unwrap();
    s.fetch(10).await.unwrap();
    drop(s);
    let q = format!("SELECT COUNT(*) FROM information_schema.processlist WHERE id = {cid}");
    let t = Instant::now();
    while all(&mut other, &q, ExecMode::Read).await[0][0] != "0" {
        assert!(t.elapsed() < Duration::from_secs(5), "query still running");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn failed_drain_is_reported_on_next_command() {
    let db = start_mysql(None, None).await;
    let mut s = editor(&db).await;
    use_test_db(&mut s).await;
    all(
        &mut s,
        "CREATE PROCEDURE p() BEGIN SELECT 1; SELECT * FROM nope; END",
        ExecMode::Other,
    )
    .await;
    s.start("CALL p()", ExecMode::Other).await.unwrap();
    assert_eq!(s.fetch(1).await.unwrap().rows.len(), 1);
    let e = s.start("SELECT 1", ExecMode::Other).await.err().unwrap();
    assert!(
        matches!(&e.kind, ErrorKind::Query { code: Some(c), .. } if c == "1146"),
        "{e:?}"
    );
    assert_eq!(all(&mut s, "SELECT 2", ExecMode::Read).await[0][0], "2");
}

#[tokio::test]
#[ignore = "docker"]
async fn kill_on_finish_never_hits_next_statement() {
    let db = start_mysql(None, None).await;
    let mut s = editor(&db).await;
    use_test_db(&mut s).await;
    with_digits(&mut s).await;
    all(&mut s, "CREATE TABLE k(a int)", ExecMode::Other).await;
    for _ in 0..20 {
        s.start(
            "SELECT a.n FROM digits a, digits b, digits c, digits d WHERE a.n >= 0 LIMIT 2000",
            ExecMode::Read,
        )
        .await
        .unwrap();
        s.fetch(1000).await.unwrap();
        s.finish().await.unwrap();
        assert_eq!(all(&mut s, "SELECT 1", ExecMode::Read).await[0][0], "1");
        s.start("INSERT INTO k VALUES (1)", ExecMode::Other)
            .await
            .unwrap();
        s.finish().await.unwrap();
    }
    assert_eq!(
        all(&mut s, "SELECT count(*) FROM k", ExecMode::Read).await[0][0],
        "20"
    );
    s.start("SELECT 1", ExecMode::Read).await.unwrap();
    assert_eq!(s.fetch(0).await.unwrap().rows.len(), 0);
    s.finish().await.unwrap();
}
