use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use std::hint::black_box;

use contime::SnapshotHistory;

mod helpers;
use helpers::{BenchContime, BenchEvent, BenchSnapshot};

const MEMORY_BUDGET_BYTES: u64 = 512 * 1024 * 1024;

fn new_event(event_id: u128, time: i64) -> BenchEvent {
    let snapshot_id = 0;
    let value = 1;

    BenchEvent::Positive(snapshot_id, time, event_id, value)
}

trait BenchHistoryApply {
    fn apply_event(&mut self, event: BenchEvent) -> i64;
}

impl BenchHistoryApply for SnapshotHistory<BenchSnapshot> {
    fn apply_event(&mut self, event: BenchEvent) -> i64 {
        self.apply_event_batch(vec![event], &mut ())
    }
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
                        history.apply_event(new_event(i, (i / 2) as i64));
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
                        history.apply_event(new_event((size - 1) - i, ((size - 1) - i) as i64));
                    }
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
                    let snap = history.snapshot_at((size / 2) as i64);
                    black_box(snap);
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn benchmark_sync_apply_end_to_end(runner: &mut Criterion) {
    let mut group = runner.benchmark_group("sync_apply_end_to_end");

    group.bench_function("fresh_lane_single_event", |bencher| {
        let contime = BenchContime::new(1, MEMORY_BUDGET_BYTES);
        let mut next_snapshot_id = 1_u128;

        bencher.iter(|| {
            let snapshot_id = next_snapshot_id;
            next_snapshot_id = next_snapshot_id.wrapping_add(1);

            contime.apply_events([BenchEvent::Positive(snapshot_id, 0, snapshot_id, 1)]).expect("single sync apply should succeed");
        });
    });

    group.finish();
}

fn benchmark_apply_orchestrator_callback(runner: &mut Criterion) {
    let mut group = runner.benchmark_group("apply_orchestrator_callback");

    group.bench_function("apply_event", |bencher| {
        let mut next_event_id = 1_u128;

        bencher.iter_batched_ref(
            || SnapshotHistory::<BenchSnapshot>::new(BenchSnapshot::default(), 0, 10000).0,
            |history| {
                next_event_id = next_event_id.wrapping_add(1);
                black_box(history.apply_event(new_event(next_event_id, next_event_id as i64)));
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("apply_event_batch_unit_context", |bencher| {
        let mut next_event_id = 1_u128;
        let mut context = ();

        bencher.iter_batched_ref(
            || SnapshotHistory::<BenchSnapshot>::new(BenchSnapshot::default(), 0, 10000).0,
            |history| {
                next_event_id = next_event_id.wrapping_add(1);
                black_box(history.apply_event_batch(vec![new_event(next_event_id, next_event_id as i64)], &mut context));
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("apply_event_batch_callback_context", |bencher| {
        let mut next_event_id = 1_u128;
        let mut context = CallbackContext::default();

        bencher.iter_batched_ref(
            || SnapshotHistory::<CallbackSnapshot>::new(CallbackSnapshot::default(), 0, 10000).0,
            |history| {
                next_event_id = next_event_id.wrapping_add(1);
                black_box(history.apply_event_batch(
                    vec![CallbackEvent { event_id: next_event_id, time: next_event_id as i64, snapshot_id: 0, value: 1 }],
                    &mut context,
                ));
                black_box(context.sink);
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

#[derive(Clone, Default, Debug, PartialEq, Eq)]
struct CallbackSnapshot {
    id: u128,
    time: i64,
    sum: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CallbackEvent {
    event_id: u128,
    time: i64,
    snapshot_id: u128,
    value: u16,
}

#[derive(Default)]
struct CallbackContext {
    sink: u128,
}

impl contime::Snapshot for CallbackSnapshot {
    type Time = i64;
    type Event = CallbackEvent;

    fn id(&self) -> u128 {
        self.id
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn set_time(&mut self, time: i64) {
        self.time = time;
    }

    fn from_event(event: &Self::Event) -> Self {
        Self { id: event.snapshot_id, time: event.time, sum: 0 }
    }

    fn conservative_size(&self) -> u64 {
        16 + 8 + 4
    }
}

impl contime::Event for CallbackEvent {
    type Time = i64;

    fn id(&self) -> u128 {
        self.event_id
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn conservative_size(&self) -> u64 {
        16 + 8 + 16 + 2
    }
}

impl contime::SnapshotEvent<CallbackSnapshot> for CallbackEvent {
    fn snapshot_id(&self) -> u128 {
        self.snapshot_id
    }
}

impl contime::ApplyEvents for CallbackSnapshot {
    fn apply_events(&mut self, batch: contime::ApplyBatch<'_, Self::Event>) {
        for event in batch.events.iter().copied() {
            self.sum += event.value as i32;
        }
        self.time = batch.time;
    }
}

impl contime::ApplyWrapper<CallbackSnapshot> for CallbackContext {
    fn apply_event_batch_wrapper(
        &mut self,
        snapshot: &mut CallbackSnapshot,
        batch: contime::ApplyBatch<'_, CallbackEvent>,
        apply_inner: contime::ApplyInner<CallbackSnapshot>,
    ) -> contime::ApplyDecision {
        let event_ids = batch.events.iter().map(|event| event.event_id).collect::<Vec<_>>();
        apply_inner.apply_event_batch(snapshot, batch);
        for event_id in event_ids {
            self.sink = self.sink.wrapping_add(event_id).wrapping_add(snapshot.sum as u128);
        }
        contime::ApplyDecision::Continue
    }
}

use pprof::criterion::{Output, PProfProfiler};

criterion_group! {
    name = benches;
    config = Criterion::default().with_profiler(PProfProfiler::new(100, Output::Flamegraph(None)));
    targets = benchmark_apply_event, benchmark_snapshot_at, benchmark_sync_apply_end_to_end, benchmark_apply_orchestrator_callback
}

criterion_main!(benches);
