use std::hint::black_box;

use contime::{RouterApplyBenchmark, SnapshotHistory, WorkerApplyBenchmark};
use criterion::measurement::WallTime;
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkGroup, Criterion};

mod helpers;
use helpers::{BenchContime, BenchEvent, BenchInputLanes, BenchSnapshotLanes};

const MEMORY_BUDGET_BYTES: u64 = 512 * 1024 * 1024;
const SNAPSHOT_ID: u128 = 1;
const HISTORY_HORIZON: i64 = 100;

fn warm_input() -> BenchInputLanes {
    BenchEvent::Positive(SNAPSHOT_ID, 1, 1, 1).into()
}

fn measured_inputs(event_count: usize) -> Vec<BenchInputLanes> {
    (0..event_count).map(|offset| BenchEvent::Positive(SNAPSHOT_ID, 2, 2 + offset as u128, 1).into()).collect()
}

fn register_api_boundary(group: &mut BenchmarkGroup<'_, WallTime>, event_count: usize) {
    group.bench_function("api", |bencher| {
        bencher.iter_batched_ref(
            || {
                let contime = BenchContime::with_history_horizon(1, MEMORY_BUDGET_BYTES, HISTORY_HORIZON);
                contime.apply([warm_input()]).expect("real benchmark warm-up should complete");
                (contime, measured_inputs(event_count))
            },
            |(contime, inputs)| {
                black_box(contime.apply(std::mem::take(inputs)).expect("benchmark apply should complete"));
            },
            BatchSize::SmallInput,
        );
    });
}

fn register_router_boundary(group: &mut BenchmarkGroup<'_, WallTime>, event_count: usize) {
    group.bench_function("router", |bencher| {
        bencher.iter_batched_ref(
            || {
                let router = RouterApplyBenchmark::<BenchSnapshotLanes, BenchInputLanes>::new(1, MEMORY_BUDGET_BYTES, HISTORY_HORIZON);
                assert!(router.apply_inputs([warm_input()]).is_empty());
                let request = router.prepare_snapshot_batches(measured_inputs(event_count));
                (router, Some(request))
            },
            |(router, request)| {
                black_box(router.apply_snapshot_batches(request.take().expect("Criterion consumes each prepared request once")));
            },
            BatchSize::SmallInput,
        );
    });
}

fn register_worker_boundary(group: &mut BenchmarkGroup<'_, WallTime>, event_count: usize) {
    group.bench_function("worker", |bencher| {
        bencher.iter_batched_ref(
            || {
                let worker = WorkerApplyBenchmark::<BenchSnapshotLanes, BenchInputLanes>::new(MEMORY_BUDGET_BYTES, HISTORY_HORIZON);
                assert!(worker.apply_inputs(SNAPSHOT_ID, [warm_input()]).is_empty());
                let batch = worker.prepare_snapshot_batch(SNAPSHOT_ID, measured_inputs(event_count));
                (worker, Some(vec![batch]))
            },
            |(worker, batch)| {
                black_box(worker.apply_snapshot_batches(batch.take().expect("Criterion consumes each prepared worker batch once")));
            },
            BatchSize::SmallInput,
        );
    });
}

fn register_snapshot_history_boundary(group: &mut BenchmarkGroup<'_, WallTime>, event_count: usize) {
    group.bench_function("snapshot_history", |bencher| {
        bencher.iter_batched_ref(
            || {
                let mut history = SnapshotHistory::<BenchSnapshotLanes>::new_with_snapshot_id(SNAPSHOT_ID, 0, HISTORY_HORIZON).0;
                history.apply_input_batch(vec![warm_input()], &mut ());
                (history, measured_inputs(event_count))
            },
            |(history, inputs)| {
                black_box(history.apply_input_batch(std::mem::take(inputs), &mut ()));
            },
            BatchSize::SmallInput,
        );
    });
}

fn benchmark_apply_boundaries(runner: &mut Criterion) {
    for event_count in [1_usize, 1_000] {
        let mut group = runner.benchmark_group(format!("apply_boundaries/{event_count}"));
        register_api_boundary(&mut group, event_count);
        register_router_boundary(&mut group, event_count);
        register_worker_boundary(&mut group, event_count);
        register_snapshot_history_boundary(&mut group, event_count);
        group.finish();
    }
}

criterion_group!(benches, benchmark_apply_boundaries);
criterion_main!(benches);
