#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[test]
fn select_is_read() {
    let c = classify("SELECT * FROM t", DriverKind::Postgres);
    assert_eq!(c.class, StatementClass::Read);
    assert!(c.cursorable);
    assert_eq!(c.exec_mode(), ExecMode::Read);
}
#[test]
fn cte_with_delete_is_dml() {
    assert!(matches!(
        classify(
            "WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d",
            DriverKind::Postgres
        )
        .class,
        StatementClass::Dml(_)
    ));
}
#[test]
fn explain_analyze_delete_is_dml() {
    let c = classify("EXPLAIN ANALYZE DELETE FROM t", DriverKind::Postgres);
    assert_eq!(c.class, StatementClass::Dml(DmlKind::Delete));
    assert!(!c.has_where);
    assert_eq!(c.target.unwrap().name, "t");
}
#[test]
fn explain_without_analyze_is_read_not_cursorable() {
    let c = classify("EXPLAIN DELETE FROM t", DriverKind::Postgres);
    assert_eq!(c.class, StatementClass::Read);
    assert!(!c.cursorable);
    assert_eq!(c.exec_mode(), ExecMode::Other);
}
#[test]
fn show_and_describe_are_read() {
    for (s, k) in [
        ("SHOW TABLES", DriverKind::Mysql),
        ("DESCRIBE t", DriverKind::Mysql),
        ("SHOW search_path", DriverKind::Postgres),
    ] {
        let c = classify(s, k);
        assert_eq!(
            (c.class, c.cursorable),
            (StatementClass::Read, false),
            "{s}"
        );
    }
}
#[test]
fn update_without_where_flagged() {
    let c = classify("UPDATE t SET a = 1", DriverKind::Mysql);
    assert!(!c.has_where);
    assert!(classify("UPDATE t SET a = 1 WHERE b = 2", DriverKind::Mysql).has_where);
}
#[test]
fn insert_values_counts_tuples() {
    assert_eq!(
        classify("INSERT INTO t VALUES (1),(2),(3)", DriverKind::Mysql).values_rows,
        Some(3)
    );
}
#[test]
fn replace_and_copy() {
    assert_eq!(
        classify("REPLACE INTO t VALUES (1)", DriverKind::Mysql).class,
        StatementClass::Dml(DmlKind::Replace)
    );
    assert_eq!(
        classify("COPY t FROM '/tmp/x'", DriverKind::Postgres).class,
        StatementClass::Dml(DmlKind::Copy)
    );
    let c = classify("COPY t TO STDOUT", DriverKind::Postgres);
    assert_eq!((c.class, c.cursorable), (StatementClass::Read, false));
}
#[test]
fn drop_truncate_alter_are_destructive() {
    for s in [
        "DROP TABLE t",
        "TRUNCATE t",
        "ALTER TABLE t DROP COLUMN a",
        "DROP SCHEMA s CASCADE",
    ] {
        let c = classify(s, DriverKind::Postgres);
        assert!(matches!(c.class, StatementClass::Destructive(_)), "{s}");
        assert!(c.is_ddl, "{s}");
    }
    assert!(matches!(
        classify("RENAME TABLE a TO b", DriverKind::Mysql).class,
        StatementClass::Destructive(DestructiveKind::Rename)
    ));
    assert!(matches!(
        classify("GRANT SELECT ON t TO u", DriverKind::Postgres).class,
        StatementClass::Destructive(DestructiveKind::Grant)
    ));
}
#[test]
fn drop_table_has_target() {
    let t = classify("DROP TABLE public.users", DriverKind::Postgres)
        .target
        .unwrap();
    assert_eq!(t.name, "users");
    assert_eq!(t.qualifier.as_deref(), Some("public"));
}
#[test]
fn create_variants() {
    assert!(matches!(
        classify("CREATE OR REPLACE VIEW v AS SELECT 1", DriverKind::Postgres).class,
        StatementClass::Destructive(DestructiveKind::CreateOrReplace)
    ));
    let c = classify("CREATE TABLE t (a int)", DriverKind::Postgres);
    assert_eq!(c.class, StatementClass::CreateSafe);
    assert!(c.is_ddl);
    assert!(matches!(
        classify("CREATE INDEX i ON t (a)", DriverKind::Postgres).class,
        StatementClass::CreateSafe
    ));
}
#[test]
fn side_effect_function_is_unknown() {
    for s in [
        "SELECT pg_terminate_backend(42)",
        "SELECT PG_ADVISORY_LOCK_shared(1)",
        "SELECT pg_catalog.nextval('s')",
        "EXPLAIN SELECT setval('s', 1)",
    ] {
        assert_eq!(
            classify(s, DriverKind::Postgres).class,
            StatementClass::Unknown,
            "{s}"
        );
    }
    assert_eq!(
        classify("SELECT GET_LOCK('a', 1)", DriverKind::Mysql).class,
        StatementClass::Unknown
    );
}
#[test]
fn call_and_unparseable_are_unknown() {
    assert_eq!(
        classify("CALL p()", DriverKind::Postgres).class,
        StatementClass::Unknown
    );
    assert_eq!(
        classify("FROBNICATE everything", DriverKind::Mysql).class,
        StatementClass::Unknown
    );
    assert_eq!(
        classify("SELECT 1; SELECT 2", DriverKind::Mysql).class,
        StatementClass::Unknown
    );
}
#[test]
fn tx_and_session() {
    assert_eq!(
        classify("START TRANSACTION", DriverKind::Mysql).class,
        StatementClass::Tx(TxOp::Begin)
    );
    assert_eq!(
        classify("COMMIT", DriverKind::Mysql).class,
        StatementClass::Tx(TxOp::Commit)
    );
    assert_eq!(
        classify("ROLLBACK", DriverKind::Postgres).class,
        StatementClass::Tx(TxOp::Rollback)
    );
    assert_eq!(
        classify("SAVEPOINT a", DriverKind::Postgres).class,
        StatementClass::Tx(TxOp::Savepoint)
    );
    assert_eq!(
        classify("SET x = 1", DriverKind::Postgres).class,
        StatementClass::Session
    );
    assert_eq!(
        classify("USE db", DriverKind::Mysql).class,
        StatementClass::Session
    );
}

mod fixes {
    use super::*;

    fn pg(s: &str) -> Classified {
        classify(s, DriverKind::Postgres)
    }
    fn my(s: &str) -> Classified {
        classify(s, DriverKind::Mysql)
    }

    #[test]
    fn parenthesized_explain_analyze_is_inner_dml() {
        assert_eq!(
            pg("EXPLAIN (ANALYZE) DELETE FROM t").class,
            StatementClass::Dml(DmlKind::Delete)
        );
        assert_eq!(
            pg("EXPLAIN (ANALYZE, FORMAT JSON) UPDATE t SET a=1").class,
            StatementClass::Dml(DmlKind::Update)
        );
        assert_eq!(
            pg("EXPLAIN (FORMAT JSON) DELETE FROM t").class,
            StatementClass::Read
        );
    }
    #[test]
    fn copy_with_nested_dml_is_dml() {
        assert!(matches!(
            pg("COPY (DELETE FROM t RETURNING *) TO STDOUT").class,
            StatementClass::Dml(DmlKind::Delete)
        ));
    }
    #[test]
    fn table_position_side_effect_fn_is_unknown() {
        for s in [
            "SELECT * FROM pg_terminate_backend(42)",
            "SELECT * FROM pg_stat_activity, LATERAL pg_terminate_backend(pid) x",
            "SELECT pg_try_advisory_lock(1)",
            "SELECT pg_advisory_xact_lock (1)",
        ] {
            assert_eq!(pg(s).class, StatementClass::Unknown, "{s}");
        }
        assert_eq!(pg("SELECT nextval FROM t").class, StatementClass::Read);
    }
    #[test]
    fn side_effect_fn_in_other_classes() {
        assert_eq!(
            pg("CREATE TABLE x AS SELECT pg_terminate_backend(pid) FROM pg_stat_activity").class,
            StatementClass::Unknown
        );
        assert_eq!(
            my("SET @a = GET_LOCK('x', 1)").class,
            StatementClass::Unknown
        );
    }
    #[test]
    fn mysql_load_data_is_dml() {
        let c = my("LOAD DATA LOCAL INFILE '/x' INTO TABLE db.t");
        assert_eq!(c.class, StatementClass::Dml(DmlKind::LoadData));
        let t = c.target.unwrap();
        assert_eq!((t.qualifier.as_deref(), t.name.as_str()), (Some("db"), "t"));
        assert_eq!(
            my("load data infile '/x' into table t").class,
            StatementClass::Dml(DmlKind::LoadData)
        );
    }
    #[test]
    fn multi_table_delete_targets_deleted_table() {
        let c = my("DELETE t2 FROM t1 JOIN t2 ON t1.a = t2.a WHERE t1.b = 1");
        assert_eq!(c.target.unwrap().name, "t2");
        assert!(c.has_where);
    }
    #[test]
    fn explain_is_never_cursorable() {
        assert!(!pg("EXPLAIN ANALYZE SELECT 1").cursorable);
    }
    #[test]
    fn copy_to_file_is_unknown_stdout_is_read() {
        assert_eq!(pg("COPY t TO '/tmp/x'").class, StatementClass::Unknown);
        assert_eq!(pg("COPY t TO PROGRAM 'cat'").class, StatementClass::Unknown);
        let c = pg("COPY t TO STDOUT");
        assert_eq!((c.class, c.cursorable), (StatementClass::Read, false));
    }
    #[test]
    fn select_into_not_cursorable() {
        let c = pg("SELECT a INTO newt FROM t");
        assert!(!c.cursorable);
    }
    #[test]
    fn grant_revoke_are_ddl() {
        assert!(pg("GRANT SELECT ON t TO u").is_ddl);
        assert!(pg("REVOKE SELECT ON t FROM u").is_ddl);
    }
}

mod round2 {
    use super::*;

    #[test]
    fn analyse_spelling() {
        assert_eq!(
            classify("EXPLAIN (ANALYSE) DELETE FROM t", DriverKind::Postgres).class,
            StatementClass::Dml(DmlKind::Delete)
        );
        let c = classify("explain analyse DELETE FROM t", DriverKind::Postgres);
        assert_eq!(c.class, StatementClass::Dml(DmlKind::Delete));
        let c = classify("EXPLAIN ANALYSE SELECT 1", DriverKind::Postgres);
        assert_eq!((c.class, c.cursorable), (StatementClass::Read, false));
    }
    #[test]
    fn downgraded_create_keeps_ddl() {
        let c = classify(
            "CREATE TABLE x AS SELECT GET_LOCK('a',1)",
            DriverKind::Mysql,
        );
        assert_eq!(c.class, StatementClass::Unknown);
        assert!(c.is_ddl);
        assert_eq!(c.target.unwrap().name, "x");
    }
}
