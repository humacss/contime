use std::hint::black_box;

use contime::{RouterApplyBenchmark, SnapshotHistory, WorkerApplyBenchmark};
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};

mod helpers;
use helpers::{BenchContime, BenchEvent, BenchInputLanes, BenchSnapshotLanes};

const EVENT_COUNT: usize = 1_000;
const MEMORY_BUDGET_BYTES: u64 = 512 * 1024 * 1024;
const SNAPSHOT_ID: u128 = 1;
const EVENT_TIME: i64 = 10;
const CURRENT_TIME: i64 = 1;
const HISTORY_HORIZON: i64 = 100;

fn inputs() -> Vec<BenchInputLanes> {
    (1..=EVENT_COUNT).map(|event_id| BenchEvent::Positive(SNAPSHOT_ID, EVENT_TIME, event_id as u128, 1).into()).collect()
}

fn benchmark_apply_boundaries(runner: &mut Criterion) {
    let mut group = runner.benchmark_group("apply_1000_events_one_snapshot");

    group.bench_function("api", |bencher| {
        bencher.iter_batched_ref(
            || {
                let contime = BenchContime::with_history_horizon(1, MEMORY_BUDGET_BYTES, HISTORY_HORIZON);
                contime.advance_to(CURRENT_TIME).expect("benchmark worker should warm up");
                (contime, inputs())
            },
            |(contime, inputs)| {
                black_box(contime.apply(std::mem::take(inputs)).expect("benchmark apply should complete"));
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("router", |bencher| {
        bencher.iter_batched_ref(
            || {
                let router = RouterApplyBenchmark::<BenchSnapshotLanes, BenchInputLanes>::new(1, MEMORY_BUDGET_BYTES, HISTORY_HORIZON);
                router.warm_up(CURRENT_TIME);
                (router, inputs())
            },
            |(router, inputs)| {
                black_box(router.apply(std::mem::take(inputs)));
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("worker", |bencher| {
        bencher.iter_batched_ref(
            || {
                let worker = WorkerApplyBenchmark::<BenchSnapshotLanes, BenchInputLanes>::new(MEMORY_BUDGET_BYTES, HISTORY_HORIZON);
                worker.warm_up(CURRENT_TIME);
                let batch = worker.prepare_batch(SNAPSHOT_ID, inputs());
                (worker, Some(batch))
            },
            |(worker, batch)| {
                black_box(worker.apply(batch.take().expect("Criterion calls each prepared worker batch once")));
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("snapshot_history", |bencher| {
        bencher.iter_batched_ref(
            || {
                let history = SnapshotHistory::<BenchSnapshotLanes>::new_with_snapshot_id(SNAPSHOT_ID, CURRENT_TIME, HISTORY_HORIZON).0;
                (history, inputs())
            },
            |(history, inputs)| {
                black_box(history.apply_input_batch(std::mem::take(inputs), &mut ()));
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(benches, benchmark_apply_boundaries);
criterion_main!(benches);
