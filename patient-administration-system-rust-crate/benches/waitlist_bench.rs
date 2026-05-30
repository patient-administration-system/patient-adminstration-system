//! waitlist_bench — Criterion benchmarks for RTT clock arithmetic.
//!
//! Measures `compute_active_weeks` at varying event counts to detect
//! regressions in the linear scan over `RTTClockEvent`s.

use chrono::{Duration, Utc};
use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use patient_administration_system::models::rtt::{RTTClockEvent, compute_active_weeks};
use uuid::Uuid;

fn make_events(count: usize) -> Vec<RTTClockEvent> {
    let pathway_id = Uuid::new_v4();
    let mut events = Vec::with_capacity(count);
    let mut t = Utc::now() - Duration::weeks(52);
    // alternating started/paused/resumed pattern
    let mut started = false;
    for i in 0..count {
        let mut e = if started {
            RTTClockEvent::paused(pathway_id, format!("reason-{i}"))
        } else if i == 0 {
            RTTClockEvent::started(pathway_id)
        } else {
            RTTClockEvent::resumed(pathway_id)
        };
        e.event_at = t;
        t += Duration::days(3);
        events.push(e);
        started = !started;
    }
    events
}

fn bench_compute_weeks(c: &mut Criterion) {
    let mut group = c.benchmark_group("compute_active_weeks");
    for size in [4, 16, 64, 256].iter() {
        let events = make_events(*size);
        group.bench_with_input(BenchmarkId::from_parameter(size), &events, |b, events| {
            b.iter(|| compute_active_weeks(black_box(events), black_box(Utc::now())));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_compute_weeks);
criterion_main!(benches);
