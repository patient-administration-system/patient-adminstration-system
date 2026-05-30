//! scheduling_bench — Criterion benchmarks for appointment overlap detection.
//!
//! Measures `TimeRange::overlaps` across a linear scan at increasing range
//! counts to detect regressions in the inner loop used by appointment
//! conflict detection.

use chrono::{Duration, Utc};
use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use patient_administration_system::models::TimeRange;

fn make_ranges(count: usize) -> Vec<TimeRange> {
    let mut ranges = Vec::with_capacity(count);
    let base = Utc::now();
    for i in 0..count {
        let start = base + Duration::minutes(i as i64 * 30);
        let end = start + Duration::minutes(30);
        ranges.push(TimeRange { start, end });
    }
    ranges
}

fn bench_overlap_scan(c: &mut Criterion) {
    let mut group = c.benchmark_group("timerange_overlap_scan");
    for size in [16, 64, 256, 1024].iter() {
        let ranges = make_ranges(*size);
        let target = TimeRange {
            start: Utc::now() + Duration::minutes(100),
            end: Utc::now() + Duration::minutes(130),
        };
        group.bench_with_input(BenchmarkId::from_parameter(size), &ranges, |b, ranges| {
            b.iter(|| {
                let mut hits = 0;
                for r in ranges.iter() {
                    if r.overlaps(black_box(&target)) {
                        hits += 1;
                    }
                }
                black_box(hits)
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_overlap_scan);
criterion_main!(benches);
