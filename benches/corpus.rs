#![allow(missing_docs)]

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use wal2json_events::{parse_v1, parse_v2};

const V2_FIXTURE: &str = include_str!("../tests/fixtures/wal2json-v2.jsonl");
const V1_FIXTURE: &str = include_str!("../tests/fixtures/wal2json-v1.json");

fn bench_parse_v2_corpus(c: &mut Criterion) {
    c.bench_function("parse_v2_corpus", |b| {
        b.iter(|| {
            for line in V2_FIXTURE.lines() {
                black_box(parse_v2(black_box(line)).unwrap());
            }
        });
    });
}

fn bench_parse_v1(c: &mut Criterion) {
    c.bench_function("parse_v1", |b| {
        b.iter(|| {
            black_box(parse_v1(black_box(V1_FIXTURE)).unwrap());
        });
    });
}

criterion_group!(benches, bench_parse_v2_corpus, bench_parse_v1);
criterion_main!(benches);
