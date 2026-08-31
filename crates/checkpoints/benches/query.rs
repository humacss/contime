use std::hint::black_box;

use contime_checkpoints::{
    query_at, ApplyBatch, ApplyEvents, CheckpointConfig, CheckpointKey, CheckpointStore, EventRef, Events, Snapshot,
};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

#[derive(Clone)]
struct TestEvent {
    id: u128,
    time: u64,
}

struct TestEvents(Vec<TestEvent>);

struct TestIter<'a>(std::slice::Iter<'a, TestEvent>);

impl<'a> Iterator for TestIter<'a> {
    type Item = EventRef<'a, u64, TestEvent>;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|event| EventRef { time: &event.time, event_id: event.id, event })
    }
}

impl Events for TestEvents {
    type Time = u64;
    type Event = TestEvent;
    type Iter<'a> = TestIter<'a>;

    fn dirty_time(&self) -> &Self::Time {
        static ZERO: u64 = 0;
        &ZERO
    }

    fn iter_after(&self, boundary: Option<&CheckpointKey<Self::Time>>) -> Self::Iter<'_> {
        let start =
            boundary.map_or(0, |boundary| self.0.partition_point(|event| (event.time, event.id) <= (boundary.time, boundary.event_id)));
        TestIter(self.0[start..].iter())
    }

    fn acknowledge_replay(&mut self) {}
}

#[derive(Clone, Default)]
struct TestSnapshot {
    time: u64,
    count: u64,
}

impl Snapshot for TestSnapshot {
    type Time = u64;

    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }
}

impl ApplyEvents<TestEvent> for TestSnapshot {
    fn create(_snapshot_id: u128, _first_event: &TestEvent) -> Self {
        Self::default()
    }

    fn apply_events(&mut self, batch: ApplyBatch<'_, Self::Time, TestEvent>) {
        self.count += batch.events.len() as u64;
    }
}

fn events(count: usize) -> TestEvents {
    TestEvents((0..count).map(|index| TestEvent { id: index as u128, time: index as u64 }).collect())
}

fn query_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("checkpoints/query");

    for count in [10_usize, 100, 1_000] {
        let events = events(count);
        let store = CheckpointStore::<TestSnapshot>::new(7, CheckpointConfig { interval: 100 });
        group.bench_with_input(BenchmarkId::new("replay_events", count), &count, |bencher, &count| {
            bencher.iter(|| black_box(query_at(black_box(&store), black_box(&events), &mut (), black_box((count - 1) as u64))));
        });
    }

    let mut retained_events = events(1_000);
    let mut retained = CheckpointStore::<TestSnapshot>::new(7, CheckpointConfig { interval: 100 });
    contime_checkpoints::replay(&mut retained, &mut retained_events, &mut ());
    group.bench_function("exact_checkpoint", |bencher| {
        bencher.iter(|| black_box(query_at(black_box(&retained), black_box(&retained_events), &mut (), black_box(999))));
    });

    group.finish();
}

criterion_group!(benches, query_benchmarks);
criterion_main!(benches);
