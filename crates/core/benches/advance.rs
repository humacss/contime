use std::time::{Duration, Instant};

use contime_core::{checkpoints, ConTime, ConTimeConfig, Input};

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
struct Time(u64);

impl checkpoints::Timestamp for Time {
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}

impl contime_worker::AdvanceTime for Time {
    fn saturating_sub(&self, retention: &Self) -> Self {
        Self(self.0.saturating_sub(retention.0))
    }
}

struct BenchEvent {
    id: u128,
    time: Time,
    snapshot_id: u128,
}

impl contime_checkpoints::Event for BenchEvent {
    type Time = Time;

    fn time(&self) -> Time {
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

impl checkpoints::Snapshot for BenchSnapshot {
    type Time = Time;

    fn time(&self) -> &Self::Time {
        &self.time
    }
    fn set_time(&mut self, time: Time) {
        self.time = time;
    }
}

impl checkpoints::ApplyEvents<BenchEvent> for BenchSnapshot {
    fn create(_snapshot_id: u128, _first_event: &BenchEvent) -> Self {
        Self::default()
    }

    fn apply_events(&mut self, batch: checkpoints::ApplyBatch<'_, '_, Time, BenchEvent>) {
        self.count += batch.events.count() as u64;
    }
}

#[derive(Clone, Copy)]
enum Workload {
    Clean,
    Anchor,
    Dirty,
}

impl Workload {
    fn name(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Anchor => "anchor",
            Self::Dirty => "dirty",
        }
    }
}

fn config(router_count: usize, worker_count: usize, dirty: bool) -> ConTimeConfig<Time> {
    ConTimeConfig {
        router_count,
        worker_count,
        placement: contime_router::Placement::default(),
        pruning_interval: std::time::Duration::ZERO,

        history_retention: Time(10),
        worker: contime_worker::WorkerConfig {
            maximum_dirty_age: Duration::from_secs(60),
            replays_per_receive: if dirty { 0 } else { 1_000 },
            deadline_compaction_minimum: 2_048,
            deadline_compaction_multiplier: 2,
        },
        checkpoints: checkpoints::CheckpointConfig { interval: 2 },
    }
}

fn events(workload: Workload) -> Vec<BenchEvent> {
    let times: &[u64] = match workload {
        Workload::Clean | Workload::Dirty => &[1],
        Workload::Anchor => &[1, 5, 10],
    };
    let mut id = 0_u128;
    let mut events = Vec::with_capacity(1_000 * times.len());
    for snapshot_id in 0..1_000_u128 {
        for time in times {
            id += 1;
            events.push(BenchEvent { id, time: Time(*time), snapshot_id });
        }
    }
    events
}

fn measure_once(router_count: usize, worker_count: usize, workload: Workload) -> Duration {
    let dirty = matches!(workload, Workload::Dirty);
    let contime = ConTime::<BenchEvent, BenchSnapshot, ()>::start(config(router_count, worker_count, dirty), ()).unwrap();
    contime.apply(events(workload)).unwrap();
    if !dirty {
        // Materialize clean fixtures without moving the retained horizon.
        contime.advance_to(Time(if matches!(workload, Workload::Anchor) { 10 } else { 1 })).unwrap();
    }
    // With the target at zero, the dirty fixture is admitted but unprocessed.
    contime.wait_until_idle(Duration::from_secs(5)).unwrap();
    assert!(contime.errors().is_empty());

    let target = if matches!(workload, Workload::Anchor) { 18 } else { 20 };

    let started = Instant::now();
    contime.advance_to(Time(target)).unwrap();
    contime.wait_until_idle(Duration::from_secs(5)).unwrap();
    let elapsed = started.elapsed();

    // Verify completed pruning, not merely delivery of the advance command.
    // Both queries, their allocations and thread shutdown are outside timing.
    let final_count = if matches!(workload, Workload::Anchor) { 3 } else { 1 };
    let anchors = contime.query_at(Time(0), 0..1_000).unwrap();
    assert!(anchors.is_empty());
    let current = contime.query_at(Time(target), 0..1_000).unwrap();
    assert_eq!(current.len(), 1_000);
    assert!(current.iter().all(|snapshot| snapshot.count == final_count));
    let retained = contime.query_events_between(0, Time(0), Time(target)).unwrap();
    let expected_times = if matches!(workload, Workload::Anchor) { vec![10] } else { vec![] };
    assert_eq!(retained.iter().map(|event| event.time.0).collect::<Vec<_>>(), expected_times);
    assert!(contime.errors().is_empty());
    contime.shutdown();
    elapsed
}

fn benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("core/advance/1000_histories");
    group.throughput(Throughput::Elements(1_000));
    for (router_count, worker_count) in [(1, 1), (1, 4), (1, 10), (2, 10)] {
        for workload in [Workload::Clean, Workload::Anchor, Workload::Dirty] {
            group.bench_with_input(
                BenchmarkId::new(workload.name(), format!("{router_count}r_{worker_count}w")),
                &(router_count, worker_count, workload),
                |bencher, (router_count, worker_count, workload)| {
                    bencher.iter_custom(|iterations| {
                        (0..iterations)
                            .map(|_| measure_once(*router_count, *worker_count, *workload))
                            .fold(Duration::ZERO, |total, elapsed| total + elapsed)
                    });
                },
            );
        }
    }
    group.finish();
}

criterion_group!(benches, benchmarks);
criterion_main!(benches);
