use std::hint::{black_box};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, BatchSize};

use contime::{SnapshotHistory};

mod helpers;
use helpers::{BenchSnapshot, BenchEvent};

fn new_event(event_id: u128, time: i64) -> BenchEvent {
    let snapshot_id = 0;
    let value = 1;

    BenchEvent::Positive(snapshot_id, time, event_id, value)
}

fn benchmark_apply_event(runner: &mut Criterion) {
    let mut group = runner.benchmark_group("apply_event");

    for size in [1_000] {
        group.bench_function(BenchmarkId::new("in_order", size), |bencher| {
            bencher.iter_batched_ref(
                || SnapshotHistory::<BenchSnapshot>::new(BenchSnapshot::default(), 0, 10000).0,
                |history| {
                    for i in 0..size {
                        history.apply_event(new_event(i, i as i64));
                    }
                },
                BatchSize::SmallInput,
            );
        });

        group.bench_function(BenchmarkId::new("out_of_order_best_case", size), |bencher| {
            bencher.iter_batched_ref(
                || {
                    let mut history = SnapshotHistory::<BenchSnapshot>::new(BenchSnapshot::default(), 0, 10000).0;
                    history.apply_event(new_event(size, size as i64));
                    history
                },
                |history| {
                    for i in 0..size {
                        history.apply_event(new_event(i as u128, i as i64));
                    }
                },
                BatchSize::SmallInput,
            );
        });

        group.bench_function(BenchmarkId::new("out_of_order_average_case", size), |bencher| {
            bencher.iter_batched_ref(
                || {
                    let mut history = SnapshotHistory::<BenchSnapshot>::new(BenchSnapshot::default(), 0, 10000).0;
                    history.apply_event(new_event(size, size as i64));
                    history
                },
                |history| {
                    for i in 0..size {
                        history.apply_event(new_event(i, (i/2) as i64));
                    }

                    black_box(&history);
                },
                BatchSize::SmallInput,
            );
        });

        group.bench_function(BenchmarkId::new("out_of_order_worst_case", size), |bencher| {
            bencher.iter_batched_ref(
                || {
                    let mut history = SnapshotHistory::<BenchSnapshot>::new(BenchSnapshot::default(), 0, 10000).0;
                    history.apply_event(new_event(size, size as i64).into());
                    history
                },
                |history| {
                    for i in 0..size {
                        history.apply_event(new_event((size-1)-i, ((size-1)-i) as i64));

                    }
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn benchmark_apply_snapshot(runner: &mut Criterion) {
    let mut group = runner.benchmark_group("apply_snapshot");

    for size in [100, 1_000] {
        group.bench_function(BenchmarkId::new("with_events", size), |bencher| {
            bencher.iter_batched_ref(
                || {
                    let mut history = SnapshotHistory::<BenchSnapshot>::new(BenchSnapshot::default(), 0, 10000).0;
                    for i in 0..size {
                        history.apply_event(new_event(i, i as i64));
                    }
                    history
                },
                |history| {
                    let snapshot = BenchSnapshot { id: 0, time: (size / 2) as i64, sum: 999 };
                    black_box(history.apply_snapshot(snapshot));
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn benchmark_snapshot_at(runner: &mut Criterion) {
    let mut group = runner.benchmark_group("snapshot_at");

    for size in [100, 1_000] {
        group.bench_function(BenchmarkId::new("query_middle", size), |bencher| {
            bencher.iter_batched_ref(
                || {
                    let mut history = SnapshotHistory::<BenchSnapshot>::new(BenchSnapshot::default(), 0, 10000).0;
                    for i in 0..size {
                        history.apply_event(new_event(i, i as i64));
                    }
                    history
                },
                |history| {
                    let (snap, _rx) = history.snapshot_at((size / 2) as i64);
                    black_box(snap);
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

use pprof::criterion::{Output, PProfProfiler};

criterion_group! {
    name = benches;
    config = Criterion::default().with_profiler(PProfProfiler::new(100, Output::Flamegraph(None)));
    targets = benchmark_apply_event, benchmark_apply_snapshot, benchmark_snapshot_at
}

criterion_main!(benches);
