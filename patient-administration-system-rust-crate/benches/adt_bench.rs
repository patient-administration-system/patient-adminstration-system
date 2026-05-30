//! adt_bench — Criterion benchmarks for ADT bed-state transitions.
//!
//! Measures a full bed-status cycle through the state machine to detect
//! regressions in `BedStatus::try_transition_to`.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use patient_administration_system::models::facility::BedStatus;

fn bench_bed_transitions(c: &mut Criterion) {
    c.bench_function("bed_status_full_cycle", |b| {
        b.iter(|| {
            let s = black_box(BedStatus::Available);
            let s = s.try_transition_to(BedStatus::Occupied).unwrap();
            let s = s.try_transition_to(BedStatus::Cleaning).unwrap();
            let _s = s.try_transition_to(BedStatus::Available).unwrap();
        });
    });
}

criterion_group!(benches, bench_bed_transitions);
criterion_main!(benches);
