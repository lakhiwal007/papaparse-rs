#![allow(clippy::unwrap_used)]

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use papaparse::{ParseConfig, Parser, parse};
use std::fmt::Write as _;
use std::hint::black_box;

fn sample(rows: usize) -> String {
    let mut s = String::from("id,name,score,active,note\n");
    for i in 0..rows {
        writeln!(
            s,
            "{i},user {i},{}.5,true,\"has, a comma and \"\"quotes\"\"\"",
            i % 100
        )
        .unwrap();
    }
    s
}

fn bench(c: &mut Criterion) {
    let input = sample(50_000);
    let mut group = c.benchmark_group("parse");
    group.throughput(Throughput::Bytes(input.len() as u64));

    group.bench_function("core", |b| {
        let cfg = ParseConfig::default().delimiter(",");
        b.iter(|| Parser::new(&cfg).unwrap().parse(black_box(&input)));
    });
    group.bench_function("auto_detect", |b| {
        let cfg = ParseConfig::default();
        b.iter(|| parse(black_box(&input), &cfg).unwrap());
    });
    group.bench_function("header_dynamic_typing", |b| {
        let cfg = ParseConfig::default().header(true).dynamic_typing(true);
        b.iter(|| parse(black_box(&input), &cfg).unwrap());
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
