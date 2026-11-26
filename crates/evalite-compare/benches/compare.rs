//! evalite against evalexpr and fasteval: compile and evaluate, measured
//! apart, on the expressions in `evalite_compare::cases`. See `README.md`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use evalite_compare::{Evalexpr, Evalite, Fasteval};

fn compare(c: &mut Criterion) {
    for case in evalite_compare::cases() {
        evalite_compare::check(&case).unwrap_or_else(|e| panic!("{e}"));
        let [evalite, evalexpr, fasteval] = &case.sources;

        {
            let mut group = c.benchmark_group(format!("compare/{}/evalite", case.name));
            let mut engine = Evalite::new().unwrap();

            group.bench_function("compile", |b| {
                b.iter(|| engine.compile(black_box(evalite)).unwrap())
            });

            let program = engine.compile(evalite).unwrap();

            group.bench_function("evaluate", |b| {
                b.iter(|| engine.evaluate(black_box(&program)).unwrap())
            });

            group.finish();
        }

        {
            let mut group = c.benchmark_group(format!("compare/{}/evalexpr", case.name));
            let engine = Evalexpr::new().unwrap();

            group.bench_function("compile", |b| {
                b.iter(|| engine.compile(black_box(evalexpr)).unwrap())
            });

            let node = engine.compile(evalexpr).unwrap();

            group.bench_function("evaluate", |b| {
                b.iter(|| engine.evaluate(black_box(&node)).unwrap())
            });

            group.finish();
        }

        {
            let mut group = c.benchmark_group(format!("compare/{}/fasteval", case.name));
            let mut engine = Fasteval::new();

            // A slab reused across iterations, cleared by each compile.
            let mut slab = fasteval::Slab::new();

            group.bench_function("compile", |b| {
                b.iter(|| engine.compile_into(black_box(fasteval), &mut slab).unwrap())
            });

            let program = engine.compile(fasteval).unwrap();

            group.bench_function("evaluate", |b| {
                b.iter(|| engine.evaluate(black_box(&program)).unwrap())
            });

            group.finish();
        }
    }
}

criterion_group!(benches, compare);
criterion_main!(benches);
