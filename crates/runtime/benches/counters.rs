//! Compare identical native insertion + live replay workloads with and without
//! `--features counters`. Startup, input preparation, verification, and shutdown
//! are outside each explicitly timed interval. Idle observation is included.
use std::hint::black_box;
use std::time::{Duration, Instant};

use contime_runtime::checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Event, Snapshot, Timestamp};
use contime_runtime::{ConTime, ConTimeConfig, Input, Placement, ThreadOutcome};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

const EVENT_COUNT: usize = 32_768;
const HISTORY_COUNT: usize = 64;
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
struct Time(u64);
impl Timestamp for Time {
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}
impl contime_worker::AdvanceTime for Time {
    fn saturating_sub(&self, retention: &Self) -> Self {
        Self(self.0.saturating_sub(retention.0))
    }
}
struct Update {
    id: u128,
    history: u128,
    time: Time,
    value: u64,
}
impl Event for Update {
    type Time = Time;
    fn time(&self) -> Time {
        self.time
    }
}
impl Input for Update {
    fn event_id(&self) -> u128 {
        self.id
    }
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        emit(self.history);
    }
}
#[derive(Clone, Default)]
struct Counter {
    time: Time,
    id: u128,
    count: u64,
    sum: u64,
}
impl Snapshot for Counter {
    type Time = Time;
    fn time(&self) -> &Time {
        &self.time
    }
    fn set_time(&mut self, time: Time) {
        self.time = time;
    }
}
impl ApplyEvents<Update> for Counter {
    fn create(id: u128, _: &Update) -> Self {
        Self { id, ..Self::default() }
    }
    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Time, Update>) {
        for event in batch.events {
            self.count += 1;
            self.sum += event.value;
        }
    }
}

#[derive(Clone, Copy)]
enum Workload {
    Ordered,
    LateWithin256,
    Duplicates,
}

impl Workload {
    fn unique_count(self) -> usize {
        match self {
            Self::Duplicates => EVENT_COUNT / 2,
            _ => EVENT_COUNT,
        }
    }

    fn event_index(self, source: usize) -> usize {
        match self {
            Self::Ordered => source,
            // Reverse each bounded arrival window, rather than the entire history.
            Self::LateWithin256 => source / 256 * 256 + (255 - source % 256),
            Self::Duplicates => source / 2,
        }
    }
}

fn trial(worker_count: usize, batch_size: usize, workload: Workload) -> Duration {
    let runtime = ConTime::<Update, Counter, ()>::start(
        ConTimeConfig {
            router_count: 1,
            worker_count,
            placement: Placement::default(),
            pruning_interval: Duration::from_secs(60),
            history_retention: Time(u64::MAX),
            checkpoints: CheckpointConfig { interval: 128 },
        },
        (),
    )
    .unwrap();
    let mut expected = [(0u64, 0u64); HISTORY_COUNT];
    for index in 0..workload.unique_count() {
        expected[index % HISTORY_COUNT].0 += 1;
        expected[index % HISTORY_COUNT].1 += (index % 7 + 1) as u64;
    }
    let mut batches = Vec::with_capacity(EVENT_COUNT.div_ceil(batch_size));
    for start in (0..EVENT_COUNT).step_by(batch_size) {
        let mut batch = Vec::with_capacity(batch_size);
        for source in start..(start + batch_size).min(EVENT_COUNT) {
            let index = workload.event_index(source);
            let history = index % HISTORY_COUNT;
            let value = (index % 7 + 1) as u64;
            batch.push(Update { id: index as u128 + 1, history: history as u128, time: Time(index as u64 + 1), value });
        }
        batches.push(batch);
    }
    // Establish a target before timing. Retention stays at zero throughout this
    // trial. Late arrivals may rebuild prefixes according to worker scheduling.
    let target = Time(EVENT_COUNT as u64 + 1);
    runtime.advance_to(target).unwrap();
    runtime.wait_until_idle(TIMEOUT).unwrap();

    let started = Instant::now();
    for batch in batches {
        runtime.apply(black_box(batch)).unwrap();
    }
    runtime.wait_until_idle(TIMEOUT).unwrap();
    let elapsed = started.elapsed();

    let reports = runtime.request_worker_status().unwrap();
    let mut statuses = Vec::with_capacity(worker_count);
    for _ in 0..worker_count {
        statuses.push(reports.recv_timeout(TIMEOUT).unwrap());
    }
    assert!(statuses.iter().all(|status| status.pending_histories == 0 && status.due_histories == 0));
    #[cfg(feature = "counters")]
    {
        let mut inserted = 0;
        let mut replayed = 0;
        for status in &statuses {
            let counts = status.counters.expect("native counters enabled");
            assert!(!counts.saturated);
            inserted += counts.inserted_applications;
            replayed += counts.replay_applications.expect("native replay count available");
        }
        assert_eq!(inserted, workload.unique_count() as u64);
        match workload {
            // Rewind work depends on how far live replay gets between arrivals.
            Workload::LateWithin256 => assert!(replayed >= inserted),
            Workload::Ordered | Workload::Duplicates => assert_eq!(replayed, inserted),
        }
    }
    #[cfg(not(feature = "counters"))]
    assert!(statuses.iter().all(|status| status.counters.is_none()));
    // Validate actual retained application state after status, outside timing.
    let snapshots = runtime.query_at(target, 0..HISTORY_COUNT as u128).unwrap();
    assert_eq!(snapshots.len(), HISTORY_COUNT);
    for snapshot in snapshots {
        assert_eq!((snapshot.count, snapshot.sum), expected[snapshot.id as usize]);
    }
    assert!(runtime.errors().is_empty());
    let shutdown = runtime.shutdown();
    assert!(shutdown.routers.iter().all(|outcome| *outcome == ThreadOutcome::Completed));
    assert!(shutdown.workers.iter().all(|outcome| *outcome == ThreadOutcome::Completed));
    elapsed
}

fn counters(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("native_counters");
    group.throughput(Throughput::Elements(EVENT_COUNT as u64));
    for workers in [1, 4] {
        for batch in [1, 1024] {
            group.bench_with_input(
                BenchmarkId::new(format!("{workers}_workers"), format!("batch_{batch}")),
                &(workers, batch),
                |b, &(workers, batch)| {
                    b.iter_custom(|iterations| (0..iterations).map(|_| trial(workers, batch, Workload::Ordered)).sum());
                },
            );
        }
    }
    group.finish();

    // Throughput counts source submissions, including duplicate attempts. The
    // duplicate case admits half as many unique events as its throughput count.
    let mut sources = criterion.benchmark_group("native_counter_source_submissions");
    sources.throughput(Throughput::Elements(EVENT_COUNT as u64));
    for (name, workload) in [("late_within_256", Workload::LateWithin256), ("duplicates_50pct", Workload::Duplicates)] {
        sources.bench_function(BenchmarkId::new(name, "1_worker_batch_1"), |b| {
            b.iter_custom(|iterations| (0..iterations).map(|_| trial(1, 1, workload)).sum());
        });
    }
    sources.finish();
}
criterion_group! {
    name = benches;
    config = Criterion::default().warm_up_time(Duration::from_secs(1)).measurement_time(Duration::from_secs(2)).sample_size(20);
    targets = counters
}
criterion_main!(benches);
