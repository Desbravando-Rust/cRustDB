use criterion::{Criterion, criterion_group, criterion_main};
use crustdb_core::dialect::{Dialect, build_table_query};
use crustdb_core::model::{ColumnMeta, DriverKind, ObjectKind, ObjectRef, TypeCategory};
use crustdb_core::query::{Sort, TableQuery};
use crustdb_core::sql::{classify, highlight, split};

struct BenchDialect;
impl Dialect for BenchDialect {
    fn kind(&self) -> DriverKind {
        DriverKind::Postgres
    }
    fn quote_ident(&self, s: &str) -> String {
        format!("\"{s}\"")
    }
    fn qualified_name(&self, o: &ObjectRef) -> String {
        self.quote_ident(&o.name)
    }
    fn param(&self, index: usize, _: &ColumnMeta) -> String {
        format!("${index}")
    }
    fn select_expr(&self, c: &ColumnMeta) -> String {
        self.quote_ident(&c.name)
    }
    fn ci_like(&self, c: &ColumnMeta, p: &str) -> String {
        format!("{} ILIKE {p}", self.as_text(c))
    }
    fn as_text(&self, c: &ColumnMeta) -> String {
        format!("{}::text", self.quote_ident(&c.name))
    }
}

fn table_query(after: Option<Vec<String>>) -> TableQuery {
    let col = |name: &str| ColumnMeta {
        name: name.into(),
        type_name: "int4".into(),
        category: TypeCategory::Integer,
        nullable: Some(false),
        is_primary_key: true,
    };
    TableQuery {
        object: ObjectRef {
            database: "db".into(),
            schema: None,
            name: "t".into(),
            kind: ObjectKind::Table,
        },
        columns: vec![col("a"), col("b")],
        filters: vec![],
        sorts: vec![
            Sort {
                column: "a".into(),
                descending: false,
            },
            Sort {
                column: "b".into(),
                descending: false,
            },
        ],
        limit: Some(301),
        offset: 300_000,
        after,
    }
}

fn bench(c: &mut Criterion) {
    let text = "SELECT a, b FROM t WHERE c = 'x;y'; -- note\n".repeat(5000);
    let offset = table_query(None);
    c.bench_function("build_table_query_offset_vs_keyset/offset", |b| {
        b.iter(|| build_table_query(&BenchDialect, &offset))
    });
    let keyset = table_query(Some(vec!["1".into(), "2".into()]));
    c.bench_function("build_table_query_offset_vs_keyset/keyset", |b| {
        b.iter(|| build_table_query(&BenchDialect, &keyset))
    });
    c.bench_function("split_5000_lines", |b| {
        b.iter(|| split(&text, DriverKind::Postgres))
    });
    c.bench_function("highlight_5000_lines", |b| {
        b.iter(|| highlight(&text, DriverKind::Postgres))
    });
    let long = "SELECT 1, 2, 3, 4; ".repeat(20000);
    c.bench_function("split_one_long_line", |b| {
        b.iter(|| split(&long, DriverKind::Postgres))
    });
    let stmts = [
        "SELECT a, b FROM t WHERE c = 1",
        "UPDATE t SET a = 1 WHERE id = 2",
        "INSERT INTO t VALUES (1),(2),(3)",
        "WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d",
        "DROP TABLE public.users",
    ];
    c.bench_function("classify_1000_statements", |b| {
        b.iter(|| {
            for i in 0..1000 {
                classify(stmts[i % stmts.len()], DriverKind::Postgres);
            }
        })
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
