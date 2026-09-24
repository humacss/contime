use std::hint::black_box;
use std::time::Duration;

use contime_core::checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Snapshot};

use contime_core::{ConTime, ConTimeConfig, Input};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
struct Time(u64);

impl contime_core::checkpoints::Timestamp for Time {
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}

impl contime_worker::AdvanceTime for Time {
    fn saturating_sub(&self, retention: &Self) -> Self {
        Self(self.0.saturating_sub(retention.0))
    }
}

const RESULT_COUNTS: [usize; 3] = [1, 100, 1_000];
const SNAPSHOT_COUNT: usize = 1_000;

struct BenchEvent {
    id: u128,
    snapshot_id: u128,
    time: Time,
}

impl contime_checkpoints::Event for BenchEvent {
    type Time = Time;

    fn time(&self) -> Self::Time {
        self.time
    }
}
impl Input for BenchEvent {
    fn event_id(&self) -> u128 {
        self.id
    }
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        emit(self.snapshot_id);
    }
}

#[derive(Clone, Default)]
struct BenchSnapshot {
    time: Time,
    count: u64,
}

impl Snapshot for BenchSnapshot {
    type Time = Time;

    fn time(&self) -> &Self::Time {
        &self.time
    }
    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }
}

impl ApplyEvents<BenchEvent> for BenchSnapshot {
    fn create(_snapshot_id: u128, _first_event: &BenchEvent) -> Self {
        Self::default()
    }

    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Self::Time, BenchEvent>) {
        self.count += batch.events.count() as u64;
    }
}

fn config(router_count: usize, worker_count: usize) -> ConTimeConfig<Time> {
    ConTimeConfig {
        router_count,
        worker_count,
        placement: contime_router::Placement::default(),
        pruning_interval: std::time::Duration::from_millis(100),

        history_retention: Time(2_000),
        worker: contime_worker::WorkerConfig {
            maximum_dirty_age: Duration::from_micros(100),
            replays_per_receive: 1,
            deadline_compaction_minimum: 1_024,
            deadline_compaction_multiplier: 2,
        },
        checkpoints: CheckpointConfig { interval: 100 },
    }
}

fn prepared_runtime(router_count: usize, worker_count: usize) -> ConTime<BenchEvent, BenchSnapshot, ()> {
    let contime = ConTime::start(config(router_count, worker_count), ()).unwrap();
    contime
        .apply((0..SNAPSHOT_COUNT).map(|index| BenchEvent { id: index as u128, snapshot_id: index as u128, time: Time(10) }).chain(
            (0..SNAPSHOT_COUNT).map(|index| BenchEvent {
                id: 10_000 + index as u128,
                snapshot_id: u128::MAX,
                time: Time(20 + index as u64),
            }),
        ))
        .unwrap();
    contime.advance_to(Time(20 + SNAPSHOT_COUNT as u64)).unwrap();
    contime.wait_until_idle(Duration::from_secs(5)).unwrap();
    assert!(contime.errors().is_empty());
    contime
}

fn query_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("core/query_end_to_end");

    for (router_count, worker_count) in [(1_usize, 1_usize), (1, 4), (1, 10), (2, 10)] {
        let contime = prepared_runtime(router_count, worker_count);
        let topology = format!("{router_count}_routers_{worker_count}_workers");
        for result_count in RESULT_COUNTS {
            let snapshot_ids = (0..result_count as u128).collect::<Vec<_>>();

            group.throughput(Throughput::Elements(result_count as u64));
            group.bench_function(BenchmarkId::new(format!("{result_count}_snapshots"), &topology), |bencher| {
                bencher.iter(|| black_box(contime.query_at(black_box(Time(10)), snapshot_ids.iter().copied()).unwrap()));
            });

            group.throughput(Throughput::Elements(result_count as u64));
            group.bench_function(BenchmarkId::new(format!("{result_count}_event_handles"), &topology), |bencher| {
                let to = 20 + result_count as u64;
                bencher.iter(|| black_box(contime.query_events_between(u128::MAX, Time(20), Time(to)).unwrap()));
            });
        }

        contime.shutdown();
    }

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
        .sample_size(20);
    targets = query_benchmarks
}
criterion_main!(benches);
