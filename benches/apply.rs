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
        self.apply_input_batch(vec![event], &mut ())
    }
}

fn benchmark_apply_event(runner: &mut Criterion) {
    let mut group = runner.benchmark_group("apply_event");
    let size = 1_000;

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
                    history.apply_event(new_event(i, i as i64));
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
                history.apply_event(new_event(size, size as i64));
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

            contime
                .apply([BenchEvent::Positive(snapshot_id, 0, snapshot_id, 1)].map(Into::into))
                .expect("single sync apply should succeed");
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

    group.bench_function("apply_input_batch_unit_context", |bencher| {
        let mut next_event_id = 1_u128;
        let mut context = ();

        bencher.iter_batched_ref(
            || SnapshotHistory::<BenchSnapshot>::new(BenchSnapshot::default(), 0, 10000).0,
            |history| {
                next_event_id = next_event_id.wrapping_add(1);
                black_box(history.apply_input_batch(vec![new_event(next_event_id, next_event_id as i64)], &mut context));
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("apply_input_batch_callback_context", |bencher| {
        let mut next_event_id = 1_u128;
        let mut context = CallbackContext::default();

        bencher.iter_batched_ref(
            || SnapshotHistory::<CallbackSnapshot>::new(CallbackSnapshot::default(), 0, 10000).0,
            |history| {
                next_event_id = next_event_id.wrapping_add(1);
                black_box(history.apply_input_batch(
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
    type Input = CallbackEvent;

    fn id(&self) -> u128 {
        self.id
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn set_time(&mut self, time: i64) {
        self.time = time;
    }

    fn conservative_size(&self) -> u64 {
        16 + 8 + 4
    }
}

impl contime::SnapshotLanes for CallbackSnapshot {
    fn materialize(snapshot_id: u128, input: &Self::Input) -> Option<Self> {
        if contime::SnapshotEvent::snapshot_id(input) != snapshot_id {
            return None;
        }

        let mut snapshot = Self::default();
        contime::SnapshotEvent::set_snapshot_identity(input, &mut snapshot);
        Some(snapshot)
    }

    fn lane_index(&self) -> usize {
        0
    }

    fn input_lane_index(snapshot_id: u128, input: &Self::Input) -> Option<usize> {
        (contime::SnapshotEvent::snapshot_id(input) == snapshot_id).then_some(0)
    }
}

impl contime::Input for CallbackEvent {
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

impl contime::Event for CallbackEvent {}

impl contime::SnapshotEvent<CallbackSnapshot> for CallbackEvent {
    fn snapshot_id(&self) -> u128 {
        self.snapshot_id
    }

    fn set_snapshot_identity(&self, snapshot: &mut CallbackSnapshot) {
        snapshot.id = self.snapshot_id;
    }
}

impl contime::ApplyEvents<CallbackEvent> for CallbackSnapshot {
    fn apply_events(&mut self, batch: contime::ApplyBatch<'_, CallbackEvent>) {
        for event in batch.events.iter().copied() {
            self.sum += event.value as i32;
        }
        self.time = batch.time;
    }
}

impl contime::ApplyWrapper<CallbackSnapshot> for CallbackContext {
    fn apply_input_batch_wrapper(
        &mut self,
        batch: contime::InputBatch<'_, CallbackEvent>,
        apply_inner: &mut contime::ApplyInner<'_, CallbackSnapshot>,
    ) {
        let event_ids = batch.inputs.iter().map(|event| event.event_id).collect::<Vec<_>>();
        apply_inner.apply_input_batch(batch);
        let snapshot = apply_inner.snapshot();
        for event_id in event_ids {
            self.sink = self.sink.wrapping_add(event_id).wrapping_add(snapshot.sum as u128);
        }
    }
}

use pprof::criterion::{Output, PProfProfiler};

criterion_group! {
    name = benches;
    config = Criterion::default().with_profiler(PProfProfiler::new(100, Output::Flamegraph(None)));
    targets = benchmark_apply_event, benchmark_snapshot_at, benchmark_sync_apply_end_to_end, benchmark_apply_orchestrator_callback
}

criterion_main!(benches);
