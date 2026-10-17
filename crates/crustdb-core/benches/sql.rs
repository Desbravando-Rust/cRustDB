use criterion::{Criterion, criterion_group, criterion_main};
use crustdb_core::model::DriverKind;
use crustdb_core::sql::{classify, split};

fn bench(c: &mut Criterion) {
    let text = "SELECT a, b FROM t WHERE c = 'x;y'; -- note\n".repeat(5000);
    c.bench_function("split_5000_lines", |b| {
        b.iter(|| split(&text, DriverKind::Postgres))
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
