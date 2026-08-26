use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use std::hint::black_box;

use contime::{ApplyBatch, ApplyEvents, SnapshotHistory};

mod helpers;
use helpers::{BenchEvent, BenchSnapshot};

fn new_event(event_id: u128, time: i64) -> BenchEvent {
    let snapshot_id = 0;
    let value = 1;

    BenchEvent::Positive(snapshot_id, time, event_id, value)
}

fn ordered_events(size: usize, snapshot_id: u128, first_id: u128, time: i64) -> Vec<BenchEvent> {
    (0..size).map(|offset| BenchEvent::Positive(snapshot_id, time, first_id + offset as u128, 1)).collect()
}

fn benchmark_snapshot_callback_same_snapshot(runner: &mut Criterion) {
    let mut group = runner.benchmark_group("snapshot_callback_same_snapshot");

    for size in [1_usize, 100, 1_000] {
        group.bench_function(BenchmarkId::from_parameter(size), |bencher| {
            bencher.iter_batched(
                || ordered_events(size, 0, 1, 0),
                |events| {
                    let mut snapshot = BenchSnapshot::default();
                    let event_refs = events.iter().collect::<Vec<_>>();
                    snapshot.apply_events(ApplyBatch { snapshot_id: 0, time: 0, history_input_count: size as u64, events: &event_refs });
                    black_box(snapshot);
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn benchmark_snapshot_history_same_snapshot(runner: &mut Criterion) {
    let mut group = runner.benchmark_group("snapshot_history_same_snapshot");

    for size in [1_usize, 100, 1_000] {
        group.bench_function(BenchmarkId::from_parameter(size), |bencher| {
            bencher.iter_batched_ref(
                || (SnapshotHistory::<BenchSnapshot>::new(BenchSnapshot::default(), 0, 10_000).0, ordered_events(size, 0, 1, 0)),
                |(history, events)| {
                    black_box(history.apply_input_batch(std::mem::take(events), &mut ()));
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn history_and_batch_for_late_rate(late_percent: u32) -> (SnapshotHistory<BenchSnapshot>, Vec<BenchEvent>, (usize, usize)) {
    let late_count = usize::try_from(late_percent).expect("late percentage fits usize") * 10;
    let ordered_count = 1_000 - late_count;
    let mut history = SnapshotHistory::<BenchSnapshot>::new(BenchSnapshot::default(), 0, 10_000).0;
    history.apply_input_batch(vec![new_event(1, 1_000)], &mut ());

    let mut batch = Vec::with_capacity(1_000);
    batch.extend((0..late_count).map(|offset| new_event(2 + offset as u128, offset as i64)));
    batch.extend((0..ordered_count).map(|offset| new_event(2 + late_count as u128 + offset as u128, 1_001 + offset as i64)));

    (history, batch, (ordered_count + 1, late_count))
}

fn benchmark_history_late_rate(runner: &mut Criterion) {
    let mut group = runner.benchmark_group("history_late_rate");

    for late_percent in [0_u32, 1, 10, 50] {
        let (mut checked_history, checked_batch, expected_counts) = history_and_batch_for_late_rate(late_percent);
        checked_history.apply_input_batch(checked_batch, &mut ());
        assert_eq!(checked_history.inputs.storage_counts(), expected_counts);

        group.bench_function(BenchmarkId::new("1000_inputs", late_percent), |bencher| {
            bencher.iter_batched_ref(
                || history_and_batch_for_late_rate(late_percent),
                |(history, batch, expected)| {
                    history.apply_input_batch(std::mem::take(batch), &mut ());
                    debug_assert_eq!(history.inputs.storage_counts(), *expected);
                    black_box(history.inputs.storage_counts());
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn benchmark_history_reverse_batch(runner: &mut Criterion) {
    let mut group = runner.benchmark_group("history_reverse_batch");
    group.bench_function("1000_inputs", |bencher| {
        bencher.iter_batched_ref(
            || {
                let mut history = SnapshotHistory::<BenchSnapshot>::new(BenchSnapshot::default(), 0, 10_000).0;
                history.apply_input_batch(vec![new_event(1, 1_000)], &mut ());
                let batch = (0..1_000).rev().map(|time| new_event(2 + time as u128, time)).collect::<Vec<_>>();
                (history, batch)
            },
            |(history, batch)| {
                history.apply_input_batch(std::mem::take(batch), &mut ());
                debug_assert_eq!(history.inputs.storage_counts(), (1, 1_000));
                black_box(history.inputs.storage_counts());
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

fn history_for_merged_replay(late_percent: u32) -> SnapshotHistory<BenchSnapshot> {
    let (mut history, batch, expected_counts) = history_and_batch_for_late_rate(late_percent);
    history.inputs.insert_batch(batch);
    assert_eq!(history.inputs.storage_counts(), expected_counts);
    history
}

fn benchmark_history_merged_replay(runner: &mut Criterion) {
    let mut group = runner.benchmark_group("history_merged_replay");

    for late_percent in [0_u32, 10, 50] {
        group.bench_function(BenchmarkId::new("1000_inputs", late_percent), |bencher| {
            bencher.iter_batched_ref(
                || history_for_merged_replay(late_percent),
                |history| {
                    black_box(history.snapshot_at(2_000));
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn history_for_horizon_prune(case: &str) -> SnapshotHistory<BenchSnapshot> {
    let initial = BenchSnapshot { id: 0, time: -1, sum: 0 };
    let mut history = SnapshotHistory::<BenchSnapshot>::new(initial, 0, 1_000).0;
    let events = match case {
        "ordered_only" => (0..1_000).map(|time| new_event(1 + time as u128, time)).collect::<Vec<_>>(),
        "late_only" => {
            history.inputs.insert_batch(vec![new_event(1, 1_000)]);
            (0..1_000).map(|time| new_event(2 + time as u128, time)).collect::<Vec<_>>()
        }
        "mixed" => {
            let mut events = (0..1_000).step_by(2).map(|time| new_event(1 + time as u128, time)).collect::<Vec<_>>();
            events.extend((1..1_000).step_by(2).map(|time| new_event(1 + time as u128, time)));
            events
        }
        _ => unreachable!("unknown pruning fixture"),
    };
    history.inputs.insert_batch(events);
    history
}

fn benchmark_history_horizon_prune(runner: &mut Criterion) {
    let mut group = runner.benchmark_group("history_horizon_prune");

    for case in ["ordered_only", "late_only", "mixed"] {
        let mut checked_history = history_for_horizon_prune(case);
        checked_history.advance(1_500);
        let expected_retained = if case == "late_only" { 501 } else { 500 };
        assert_eq!(checked_history.inputs.len(), expected_retained);

        group.bench_function(case, |bencher| {
            bencher.iter_batched_ref(
                || history_for_horizon_prune(case),
                |history| {
                    black_box(history.advance(1_500));
                    black_box(history.inputs.storage_counts());
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
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
    targets =
        benchmark_apply_event,
        benchmark_snapshot_at,
        benchmark_apply_orchestrator_callback,
        benchmark_snapshot_callback_same_snapshot,
        benchmark_snapshot_history_same_snapshot,
        benchmark_history_late_rate,
        benchmark_history_reverse_batch,
        benchmark_history_merged_replay,
        benchmark_history_horizon_prune
}

criterion_main!(benches);
