use std::hint::black_box;
use std::sync::Arc;

use contime_events::{Event, EventHistory};
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};

const EVENT_COUNT: usize = 1_000;

#[repr(C, align(16))]
struct BenchmarkEvent<const PAYLOAD_BYTES: usize> {
    id: u128,
    time: i64,
    payload: [u8; PAYLOAD_BYTES],
}

impl<const PAYLOAD_BYTES: usize> Event for BenchmarkEvent<PAYLOAD_BYTES> {
    type Time = i64;

    fn event_id(&self) -> u128 {
        self.id
    }

    fn time(&self) -> Self::Time {
        self.time
    }
}

#[derive(Clone)]
struct SharedEvent<const PAYLOAD_BYTES: usize>(Arc<BenchmarkEvent<PAYLOAD_BYTES>>);

impl<const PAYLOAD_BYTES: usize> Event for SharedEvent<PAYLOAD_BYTES> {
    type Time = i64;

    fn event_id(&self) -> u128 {
        self.0.id
    }

    fn time(&self) -> Self::Time {
        self.0.time
    }
}

const _: () = assert!(std::mem::size_of::<BenchmarkEvent<8>>() == 32);
const _: () = assert!(std::mem::size_of::<BenchmarkEvent<40>>() == 64);
const _: () = assert!(std::mem::size_of::<BenchmarkEvent<184>>() == 208);
const _: () = assert!(std::mem::size_of::<BenchmarkEvent<984>>() == 1_008);

fn events<const PAYLOAD_BYTES: usize>() -> Vec<SharedEvent<PAYLOAD_BYTES>> {
    (0..EVENT_COUNT)
        .map(|index| SharedEvent(Arc::new(BenchmarkEvent { id: index as u128, time: index as i64, payload: [index as u8; PAYLOAD_BYTES] })))
        .collect()
}

fn ordered_fixture<const PAYLOAD_BYTES: usize>() -> (EventHistory<SharedEvent<PAYLOAD_BYTES>>, Vec<SharedEvent<PAYLOAD_BYTES>>) {
    (EventHistory::with_capacity(EVENT_COUNT), events())
}

fn late_fixture<const PAYLOAD_BYTES: usize>() -> (EventHistory<SharedEvent<PAYLOAD_BYTES>>, Vec<SharedEvent<PAYLOAD_BYTES>>) {
    let mut history = EventHistory::with_capacity(EVENT_COUNT + 1);
    history.insert(SharedEvent(Arc::new(BenchmarkEvent {
        id: EVENT_COUNT as u128,
        time: EVENT_COUNT as i64,
        payload: [0; PAYLOAD_BYTES],
    })));
    (history, events())
}

fn duplicate_fixture<const PAYLOAD_BYTES: usize>() -> (EventHistory<SharedEvent<PAYLOAD_BYTES>>, Vec<SharedEvent<PAYLOAD_BYTES>>) {
    let events = events();
    let mut history = EventHistory::with_capacity(EVENT_COUNT);
    for event in &events {
        history.insert(event.clone());
    }
    (history, events)
}

fn benchmark_insertion_workload<const PAYLOAD_BYTES: usize>(
    criterion: &mut Criterion,
    event_size: usize,
    workload: &str,
    fixture: fn() -> (EventHistory<SharedEvent<PAYLOAD_BYTES>>, Vec<SharedEvent<PAYLOAD_BYTES>>),
) {
    let mut group = criterion.benchmark_group(format!("events/insertion/{workload}"));
    group.throughput(Throughput::Elements(EVENT_COUNT as u64));
    group.bench_with_input(BenchmarkId::from_parameter(format!("{event_size}_bytes")), &event_size, |bencher, _event_size| {
        bencher.iter_batched(
            fixture,
            |(mut history, mut events)| {
                for event in events.drain(..) {
                    black_box(history.insert(event));
                }
                black_box((history, events))
            },
            BatchSize::LargeInput,
        );
    });
    group.finish();
}

fn benchmark_insertion_size<const PAYLOAD_BYTES: usize>(criterion: &mut Criterion, event_size: usize) {
    benchmark_insertion_workload(criterion, event_size, "ordered", ordered_fixture::<PAYLOAD_BYTES>);
    benchmark_insertion_workload(criterion, event_size, "late", late_fixture::<PAYLOAD_BYTES>);
    benchmark_insertion_workload(criterion, event_size, "duplicate", duplicate_fixture::<PAYLOAD_BYTES>);
}

fn insertion_matrix(criterion: &mut Criterion) {
    benchmark_insertion_size::<8>(criterion, 32);
    benchmark_insertion_size::<40>(criterion, 64);
    benchmark_insertion_size::<184>(criterion, 208);
    benchmark_insertion_size::<984>(criterion, 1_008);
}

fn populated_history() -> EventHistory<SharedEvent<184>> {
    let mut history = EventHistory::with_capacity(EVENT_COUNT);
    for event in events() {
        history.insert(event);
    }
    history
}

fn iteration(criterion: &mut Criterion) {
    let history = populated_history();
    let mut group = criterion.benchmark_group("events/iteration/208_byte_events");
    group.throughput(Throughput::Elements(EVENT_COUNT as u64));

    group.bench_function("full_history", |bencher| {
        bencher.iter(|| {
            let mut count = 0;
            for entry in history.iter() {
                black_box(entry);
                count += 1;
            }
            black_box(count)
        });
    });

    group.bench_function("from_dirty", |bencher| {
        bencher.iter(|| {
            let mut count = 0;
            for entry in history.iter_from_dirty() {
                black_box(entry);
                count += 1;
            }
            black_box(count)
        });
    });

    group.finish();
}

criterion_group!(benches, insertion_matrix, iteration);
criterion_main!(benches);
