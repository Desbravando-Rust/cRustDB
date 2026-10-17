use criterion::{Criterion, criterion_group, criterion_main};
use crustdb_core::model::DriverKind;
use crustdb_core::sql::split;

fn bench(c: &mut Criterion) {
    let text = "SELECT a, b FROM t WHERE c = 'x;y'; -- note\n".repeat(5000);
    c.bench_function("split_5000_lines", |b| {
        b.iter(|| split(&text, DriverKind::Postgres))
    });
    let long = "SELECT 1, 2, 3, 4; ".repeat(20000);
    c.bench_function("split_one_long_line", |b| {
        b.iter(|| split(&long, DriverKind::Postgres))
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
