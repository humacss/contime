use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

use contime_router::{route, InputBatch, RoutableInput, RoutedInput, WorkerBatch};
use criterion::measurement::WallTime;
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkGroup, Criterion};
use crossbeam_channel::{unbounded, Receiver, Sender};
use pprof::criterion::{Output, PProfProfiler};

const BATCH_COUNT: usize = 100;
const INPUTS_PER_BATCH: usize = 1_000;
const WORKER_COUNT: usize = 8;
const SEED: u64 = 7;

enum SnapshotIds {
    One([u128; 1]),
    Two([u128; 2]),
    Three([u128; 3]),
}

impl SnapshotIds {
    fn visit(&self, emit: &mut impl FnMut(u128)) {
        match self {
            Self::One(snapshot_ids) => snapshot_ids.iter().copied().for_each(emit),
            Self::Two(snapshot_ids) => snapshot_ids.iter().copied().for_each(emit),
            Self::Three(snapshot_ids) => snapshot_ids.iter().copied().for_each(emit),
        }
    }
}

#[repr(C)]
struct BenchmarkEvent<const PAYLOAD_BYTES: usize> {
    snapshot_ids: SnapshotIds,
    payload: [u8; PAYLOAD_BYTES],
}

impl<const PAYLOAD_BYTES: usize> RoutableInput for BenchmarkEvent<PAYLOAD_BYTES> {
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        self.snapshot_ids.visit(emit);
    }
}

#[repr(C)]
struct BenchmarkEvent32 {
    first_snapshot_id: u128,
    payload: [u8; 15],
    route_count: u8,
}

impl RoutableInput for BenchmarkEvent32 {
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        (0..self.route_count).for_each(|route_index| emit(self.first_snapshot_id + u128::from(route_index)));
    }
}

#[repr(C)]
struct ArcAllocationValue<const BYTES: usize> {
    bytes: [u8; BYTES],
}

const _: () = assert!(std::mem::size_of::<BenchmarkEvent32>() == 32);
const _: () = assert!(std::mem::size_of::<BenchmarkEvent<0>>() == 64);
const _: () = assert!(std::mem::size_of::<BenchmarkEvent<144>>() == 208);
const _: () = assert!(std::mem::size_of::<BenchmarkEvent<944>>() == 1_008);
const _: () = assert!(std::mem::size_of::<RoutedInput<BenchmarkEvent32>>() == 32);
const _: () = assert!(std::mem::size_of::<RoutedInput<BenchmarkEvent<0>>>() == 32);

type Completion = Sender<()>;
type Fixture<I> = (
    Receiver<InputBatch<I, Completion>>,
    Vec<Sender<WorkerBatch<I, Completion>>>,
    Vec<Receiver<WorkerBatch<I, Completion>>>,
    Vec<Receiver<()>>,
);

fn fixture<I>(inputs: Vec<Arc<I>>) -> Fixture<I> {
    fixture_with_worker_count(inputs, WORKER_COUNT)
}

fn fixture_with_worker_count<I>(inputs: Vec<Arc<I>>, worker_count: usize) -> Fixture<I> {
    let (input_sender, input_receiver) = unbounded();
    let mut completion_receivers = Vec::with_capacity(BATCH_COUNT);

    for _ in 0..BATCH_COUNT {
        let (completion, completion_receiver) = unbounded();
        input_sender.send(InputBatch { inputs: inputs.clone(), completion }).unwrap();
        completion_receivers.push(completion_receiver);
    }
    drop(input_sender);

    let mut worker_outputs = Vec::with_capacity(worker_count);
    let mut worker_receivers = Vec::with_capacity(worker_count);
    for _ in 0..worker_count {
        let (worker_sender, worker_receiver) = unbounded();
        worker_outputs.push(worker_sender);
        worker_receivers.push(worker_receiver);
    }

    (input_receiver, worker_outputs, worker_receivers, completion_receivers)
}

fn snapshot_ids(input_index: usize, route_count: usize) -> SnapshotIds {
    let first = input_index as u128;
    match route_count {
        1 => SnapshotIds::One([first]),
        2 => SnapshotIds::Two([first, first + 1]),
        3 => SnapshotIds::Three([first, first + 1, first + 2]),
        _ => unreachable!("the integration matrix uses one to three routes"),
    }
}

fn benchmark_events<const PAYLOAD_BYTES: usize>(route_count: usize) -> impl Iterator<Item = BenchmarkEvent<PAYLOAD_BYTES>> {
    (0..INPUTS_PER_BATCH).map(move |input_index| BenchmarkEvent {
        snapshot_ids: snapshot_ids(input_index, route_count),
        payload: [input_index as u8; PAYLOAD_BYTES],
    })
}

fn arc_fixture<const PAYLOAD_BYTES: usize>(route_count: usize) -> Fixture<BenchmarkEvent<PAYLOAD_BYTES>> {
    fixture(benchmark_events::<PAYLOAD_BYTES>(route_count).map(Arc::new).collect())
}

fn benchmark_event32(input_index: usize, route_count: usize) -> BenchmarkEvent32 {
    BenchmarkEvent32 {
        first_snapshot_id: input_index as u128,
        payload: [input_index as u8; 15],
        route_count: route_count.try_into().expect("the benchmark route count fits in u8"),
    }
}

fn benchmark_32_byte_event(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("router/100_batches/1000_inputs/32_byte_events/8_workers");
    for route_count in 1..=3 {
        let route_label = if route_count == 1 { String::from("1_route") } else { format!("{route_count}_routes") };
        group.bench_function(route_label, |bencher| {
            bencher.iter_batched(
                || fixture((0..INPUTS_PER_BATCH).map(|input_index| Arc::new(benchmark_event32(input_index, route_count))).collect()),
                |(input_receiver, worker_outputs, worker_receivers, completion_receivers)| {
                    route(SEED, input_receiver, &worker_outputs).unwrap();
                    black_box((worker_outputs, worker_receivers, completion_receivers))
                },
                BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

fn benchmark_arc_new_for_size<const BYTES: usize>(group: &mut BenchmarkGroup<'_, WallTime>, event_size: usize) {
    group.bench_function(format!("{event_size}_bytes"), |bencher| {
        bencher.iter_batched(|| ArcAllocationValue::<BYTES> { bytes: [black_box(7); BYTES] }, Arc::new, BatchSize::SmallInput);
    });
}

fn benchmark_arc_new(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("arc_new");
    benchmark_arc_new_for_size::<32>(&mut group, 32);
    benchmark_arc_new_for_size::<64>(&mut group, 64);
    benchmark_arc_new_for_size::<208>(&mut group, 208);
    benchmark_arc_new_for_size::<1_008>(&mut group, 1_008);
    group.finish();
}

fn benchmark_single_worker(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("router/100_batches/1000_inputs/64_byte_events/1_worker");
    group.bench_function("1_route", |bencher| {
        bencher.iter_batched(
            || fixture_with_worker_count(benchmark_events::<0>(1).map(Arc::new).collect(), 1),
            |(input_receiver, worker_outputs, worker_receivers, completion_receivers)| {
                route(SEED, input_receiver, &worker_outputs).unwrap();
                black_box((worker_outputs, worker_receivers, completion_receivers))
            },
            BatchSize::LargeInput,
        );
    });
    group.finish();
}

fn benchmark_matrix<const PAYLOAD_BYTES: usize>(criterion: &mut Criterion, event_size: usize) {
    let mut group = criterion.benchmark_group(format!("router/100_batches/1000_inputs/{event_size}_byte_events/8_workers"));
    for route_count in 1..=3 {
        let route_label = if route_count == 1 { String::from("1_route") } else { format!("{route_count}_routes") };
        group.bench_function(route_label, |bencher| {
            bencher.iter_batched(
                || arc_fixture::<PAYLOAD_BYTES>(route_count),
                |(input_receiver, worker_outputs, worker_receivers, completion_receivers)| {
                    route(SEED, input_receiver, &worker_outputs).unwrap();
                    black_box((worker_outputs, worker_receivers, completion_receivers))
                },
                BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

fn sustained_router(criterion: &mut Criterion) {
    benchmark_32_byte_event(criterion);
    benchmark_matrix::<0>(criterion, 64);
    benchmark_matrix::<144>(criterion, 208);
    benchmark_matrix::<944>(criterion, 1_008);
    benchmark_single_worker(criterion);
    benchmark_arc_new(criterion);
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(2))
        .measurement_time(Duration::from_secs(5))
        .sample_size(50)
        .with_profiler(PProfProfiler::new(1_000, Output::Flamegraph(None)));
    targets = sustained_router
}
criterion_main!(benches);
