use contime_snapshots::{Apply, Checkpoint, Event, EventStore, Snapshot, SnapshotStore, Timestamp};
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use std::{hint::black_box, time::Duration};

const EVENT_COUNT: u64 = 1_000;
const INTERVAL: u64 = 100;
const STEP: u64 = 100;
const RETAINED_WINDOW: u64 = 50;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Time(u64);
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct TestSnapshot {
    time: Time,
    sum: u64,
}
struct TestEvent(Time, u64);
struct TestEventStore(Vec<TestEvent>);
type Store = SnapshotStore<TestSnapshot, TestEventStore>;

impl Timestamp for Time {
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}
impl Snapshot for TestSnapshot {
    type Time = Time;
    fn time(&self) -> &Time {
        &self.time
    }
    fn set_time(&mut self, time: Time) {
        self.time = time;
    }
}
impl Event for TestEvent {
    type Time = Time;
    fn time(&self) -> Time {
        self.0
    }
}
impl EventStore for TestEventStore {
    type Time = Time;
    type Event = TestEvent;
    type Iter<'a> = std::slice::Iter<'a, TestEvent>;
    fn iter_after(&self, boundary: Option<&Time>) -> Self::Iter<'_> {
        let start = boundary.map_or(0, |time| self.0.partition_point(|event| event.0 <= *time));
        self.0[start..].iter()
    }
    fn prune_before(&mut self, horizon: &Time) {
        let end = self.0.partition_point(|event| event.0 < *horizon);
        self.0.drain(..end);
    }
}
impl Apply<Checkpoint<TestSnapshot>> for TestEvent {
    fn apply<'a>(checkpoint: &mut Checkpoint<TestSnapshot>, events: impl Iterator<Item = &'a Self>, _: &()) {
        checkpoint.snapshot.sum += events.map(|event| event.1).sum::<u64>();
    }
}
fn events(count: u64) -> TestEventStore {
    TestEventStore((1..=count).map(|time| TestEvent(Time(time), time)).collect())
}
fn store(count: u64) -> Store {
    Store::new(events(count), TestSnapshot::default(), INTERVAL)
}
fn hook(snapshot: &mut TestSnapshot, time: &Time, _: &()) {
    black_box((snapshot, time));
}
fn prune_ready() -> Store {
    let mut store = store(EVENT_COUNT);
    store.replay(Time(EVENT_COUNT), &()).unwrap();
    store.forward(Time(EVENT_COUNT / 2 + 1), &(), hook).unwrap();
    store
}
fn cycle(store: &mut Store, target: u64) -> Box<TestSnapshot> {
    let result = store.query(Time(target), &()).unwrap();
    store.replay(Time(target), &()).unwrap();
    store.forward(Time(target - RETAINED_WINDOW + 1), &(), hook).unwrap();
    store.prune();
    result
}
fn expected(time: u64) -> TestSnapshot {
    TestSnapshot { time: Time(time), sum: time * (time + 1) / 2 }
}

// Run the exact fixtures through correctness checks before any measurements.
fn verify() {
    let mut fresh = store(EVENT_COUNT);
    assert_eq!(*fresh.query(Time(0), &()).unwrap(), TestSnapshot::default());
    assert_eq!(*fresh.query(Time(EVENT_COUNT), &()).unwrap(), expected(EVENT_COUNT));
    fresh.replay(Time(EVENT_COUNT), &()).unwrap();
    assert_eq!(*fresh.query(Time(EVENT_COUNT), &()).unwrap(), expected(EVENT_COUNT));
    let mut forwarded = store(EVENT_COUNT);
    forwarded.forward(Time(EVENT_COUNT + 1), &(), hook).unwrap();
    assert_eq!(*forwarded.query(Time(EVENT_COUNT + 1), &()).unwrap(), expected(EVENT_COUNT));
    let mut pruned = prune_ready();
    pruned.prune();
    assert_eq!(*pruned.query(Time(EVENT_COUNT), &()).unwrap(), expected(EVENT_COUNT));
    assert!(pruned.query(Time(EVENT_COUNT / 2), &()).is_err());
    for cycles in [10, 100] {
        let mut mixed = store(cycles * STEP);
        for round in 1..=cycles {
            let target = round * STEP;
            assert_eq!(*cycle(&mut mixed, target), expected(target));
            assert_eq!(*mixed.query(Time(target), &()).unwrap(), expected(target));
            let horizon = target - RETAINED_WINDOW + 1;
            assert_eq!(*mixed.query(Time(horizon), &()).unwrap(), expected(horizon));
        }
    }
}

fn benchmarks(c: &mut Criterion) {
    verify();
    let mut group = c.benchmark_group("snapshot_store");
    group.bench_function("new", |b| {
        b.iter_batched(
            || (events(EVENT_COUNT), TestSnapshot::default()),
            |(events, snapshot)| black_box(Store::new(black_box(events), black_box(snapshot), black_box(INTERVAL))),
            BatchSize::SmallInput,
        )
    });
    group.bench_function("query/1000_events", |b| {
        b.iter_batched_ref(
            || store(EVENT_COUNT),
            |store| {
                black_box(black_box(store).query(black_box(Time(EVENT_COUNT)), black_box(&())).unwrap());
            },
            BatchSize::SmallInput,
        )
    });
    group.bench_function("replay/1000_events", |b| {
        b.iter_batched_ref(
            || store(EVENT_COUNT),
            |store| {
                black_box(&mut *store).replay(black_box(Time(EVENT_COUNT)), black_box(&())).unwrap();
                black_box(store);
            },
            BatchSize::SmallInput,
        )
    });
    group.bench_function("forward/1000_events", |b| {
        b.iter_batched_ref(
            || store(EVENT_COUNT),
            |store| {
                black_box(&mut *store).forward(black_box(Time(EVENT_COUNT + 1)), black_box(&()), hook).unwrap();
                black_box(store);
            },
            BatchSize::SmallInput,
        )
    });
    group.bench_function("prune/500_of_1000_events", |b| {
        b.iter_batched_ref(
            prune_ready,
            |store| {
                black_box(&mut *store).prune();
                black_box(store);
            },
            BatchSize::SmallInput,
        )
    });
    for cycles in [10, 100] {
        group.bench_with_input(BenchmarkId::new("mixed_cycles", cycles), &cycles, |b, &cycles| {
            b.iter_batched_ref(
                || store(cycles * STEP),
                |store| {
                    for round in 1..=cycles {
                        black_box(cycle(black_box(&mut *store), black_box(round * STEP)));
                    }
                    black_box(store);
                },
                BatchSize::SmallInput,
            )
        });
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(30).warm_up_time(Duration::from_millis(200)).measurement_time(Duration::from_secs(1));
    targets = benchmarks
}
criterion_main!(benches);
