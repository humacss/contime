//! Matched borrowed-application and managed-pipeline cost, without event generation in the timing.
use contime::checkpoints::{ApplyBatch, ApplyEvents};
use contime::prelude::*;
use criterion::{black_box, criterion_group, Criterion, Throughput};
use std::time::{Duration, Instant};

struct Added {
    time: i64,
    id: u128,
    owner: u128,
    value: u64,
}
impl contime::checkpoints::Event for Added {
    type Time = i64;
    fn time(&self) -> Self::Time {
        self.time
    }
}

struct Bonus {
    time: i64,
    id: u128,
    owner: u128,
    value: u64,
}
impl contime::checkpoints::Event for Bonus {
    type Time = i64;
    fn time(&self) -> Self::Time {
        self.time
    }
}

impl EventId for Added {
    fn event_id(&self) -> u128 {
        self.id
    }
}
impl EventId for Bonus {
    fn event_id(&self) -> u128 {
        self.id
    }
}
#[derive(Clone, Default)]

struct Total {
    time: i64,
    owner: u128,
    value: u64,
}
impl contime::checkpoints::Snapshot for Total {
    type Time = i64;
    fn time(&self) -> &Self::Time {
        &self.time
    }
    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }
}

impl SnapshotId for Total {
    fn snapshot_id(&self) -> u128 {
        self.owner
    }
}
impl SnapshotApply for Total {
    fn define(events: &mut EventSetBuilder<Self>) {
        events.event::<Added>(|e, emit| emit(e.owner)).event::<Bonus>(|e, emit| emit(e.owner));
    }
    fn create(owner: u128, _: EventRef<'_>) -> Self {
        Self { owner, ..Self::default() }
    }
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        for event in batch.events::<Added>() {
            self.value += event.value;
        }
        for event in batch.events::<Bonus>() {
            self.value += event.value;
        }
    }
}
#[derive(ContimeModule)]
#[contime_module(time=i64,events=[Added,Bonus],snapshots=[Total])]
struct App;
fn events(count: usize, snapshots: usize) -> Vec<AppEventLanes> {
    (0..count)
        .map(|i| {
            let time = (i / snapshots) as i64;
            let id = i as u128 + 1;
            let owner = (i % snapshots) as u128;
            if i % 2 == 0 {
                Added { time, id, owner, value: 1 }.into()
            } else {
                Bonus { time, id, owner, value: 2 }.into()
            }
        })
        .collect()
}
fn benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("module_application/direct");
    for size in [1, 8, 64] {
        let mut inputs = events(size, 1);
        for e in &mut inputs {
            match e {
                AppEventLanes::Added(e) => e.time = 0,
                AppEventLanes::Bonus(e) => e.time = 0,
            }
        }
        let mut state = AppSnapshotLanes::Total(Total::default());
        group.throughput(Throughput::Elements(size as u64));
        group.bench_function(size.to_string(), |b| {
            b.iter(|| {
                let mut events = black_box(&inputs).iter();
                state.apply_events(ApplyBatch { snapshot_id: 0, time: 0, history_event_count: 0, events: &mut events });
                black_box(&state);
            })
        });
    }
    group.finish();
    let mut group = c.benchmark_group("module_application/runtime");
    const COUNT: usize = 65536;
    const SNAPSHOTS: usize = 256;
    group.throughput(Throughput::Elements(COUNT as u64));
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(2));
    group.warm_up_time(Duration::from_secs(1));
    for workers in [1, 4] {
        group.bench_function(workers.to_string(), |b| {
            b.iter_custom(|iterations| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iterations {
                    let inputs = events(COUNT, SNAPSHOTS);
                    let contime = ConTime::<App>::start(ConTimeConfig {
                        router_count: 1,
                        worker_count: workers,
                        placement: Placement::default(),
                        pruning_interval: Duration::from_secs(60),
                        history_retention: i64::MAX,
                        checkpoints: CheckpointConfig { interval: 128 },
                    })
                    .unwrap();
                    let start = Instant::now();
                    let mut inputs = inputs.into_iter();
                    loop {
                        let batch = inputs.by_ref().take(1024).collect::<Vec<_>>();
                        if batch.is_empty() {
                            break;
                        }
                        contime.apply(batch).unwrap();
                    }
                    contime.advance_to((COUNT / SNAPSHOTS) as i64).unwrap();
                    contime.wait_until_idle(Duration::from_secs(10)).unwrap();
                    elapsed += start.elapsed();
                    let states = contime.query_at((COUNT / SNAPSHOTS) as i64, 0..SNAPSHOTS as u128).unwrap();
                    let sum: u64 = states
                        .iter()
                        .map(|s| {
                            let AppSnapshotLanes::Total(s) = &**s;
                            s.value
                        })
                        .sum();
                    assert_eq!(sum, (COUNT / 2 * 3) as u64);
                    assert!(contime.errors().is_empty());
                    contime.shutdown();
                }
                elapsed
            })
        });
    }
    group.finish();
}
#[path = "module_application/isolation.rs"]
mod isolation;
criterion_group!(benches, benchmarks, isolation::benchmarks);
fn main() {
    if std::env::var_os("CONTIME_PERF_PAIRED").is_some() {
        isolation::paired();
    } else {
        benches();
        Criterion::default().configure_from_args().final_summary();
    }
}
