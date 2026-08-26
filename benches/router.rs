use std::hint::black_box;

use contime::{CompletionBenchmark, Input, InputRoute, Marker, RoutePartitionBenchmark, TestEvent, TestInputLanes, TestSnapshotLanes};
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MultiRouteMarker {
    event_id: u128,
    snapshot_ids: [u128; 3],
}

impl Input for MultiRouteMarker {
    type Time = i64;

    fn id(&self) -> u128 {
        self.event_id
    }

    fn time(&self) -> Self::Time {
        10
    }

    fn conservative_size(&self) -> u64 {
        size_of::<Self>() as u64
    }
}

impl Marker for MultiRouteMarker {}

impl InputRoute for MultiRouteMarker {
    fn visit_snapshot_ids<F>(&self, visit: &mut F)
    where
        F: FnMut(u128),
    {
        for snapshot_id in self.snapshot_ids {
            visit(snapshot_id);
        }
    }
}

contime::lanes! {
    mod multi_route_lanes;
    snapshots [contime::TestSnapshot];
    markers [MultiRouteMarker];
    routes [
        TestEvent => [contime::TestSnapshot],
    ];
}

fn single_target_inputs(count: usize, separate_snapshots: bool) -> Vec<TestInputLanes> {
    (0..count)
        .map(|event_id| TestEvent::Positive(if separate_snapshots { event_id as u128 + 1 } else { 7 }, 10, event_id as u128, 1).into())
        .collect()
}

fn multi_target_inputs(count: usize) -> Vec<multi_route_lanes::InputLanes> {
    (0..count).map(|event_id| MultiRouteMarker { event_id: event_id as u128, snapshot_ids: [7, 11, 19] }.into()).collect()
}

fn router_partition(c: &mut Criterion) {
    let mut group = c.benchmark_group("router_partition");
    for count in [1, 100, 1_000] {
        let one_worker = RoutePartitionBenchmark::new(1);
        group.bench_with_input(BenchmarkId::new("single_target_one_worker", count), &count, |b, &count| {
            b.iter_batched(
                || one_worker.prepare::<TestSnapshotLanes, TestInputLanes, _>(single_target_inputs(count, false)),
                |batches| black_box(one_worker.partition(batches)),
                BatchSize::SmallInput,
            );
        });

        let eight_workers = RoutePartitionBenchmark::new(8);
        group.bench_with_input(BenchmarkId::new("single_target_eight_workers", count), &count, |b, &count| {
            b.iter_batched(
                || eight_workers.prepare::<TestSnapshotLanes, TestInputLanes, _>(single_target_inputs(count, true)),
                |batches| black_box(eight_workers.partition(batches)),
                BatchSize::SmallInput,
            );
        });
        group.bench_with_input(BenchmarkId::new("multi_target_eight_workers", count), &count, |b, &count| {
            b.iter_batched(
                || eight_workers.prepare::<multi_route_lanes::SnapshotLanes, multi_route_lanes::InputLanes, _>(multi_target_inputs(count)),
                |batches| black_box(eight_workers.partition(batches)),
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn router_enqueue(c: &mut Criterion) {
    let mut group = c.benchmark_group("router_enqueue");
    for count in [1, 100, 1_000] {
        let (tx, rx) = crossbeam_channel::unbounded::<Vec<TestInputLanes>>();
        let consumer = std::thread::spawn(move || while rx.recv().is_ok() {});
        group.bench_with_input(BenchmarkId::new("one_worker", count), &count, |b, &count| {
            b.iter_batched(|| single_target_inputs(count, false), |inputs| tx.send(black_box(inputs)).unwrap(), BatchSize::SmallInput);
        });
        drop(tx);
        consumer.join().unwrap();
    }
    group.finish();
}

fn api_completion(c: &mut Criterion) {
    let mut group = c.benchmark_group("api_completion");
    for worker_count in [1, 2, 8] {
        group.bench_with_input(BenchmarkId::new("empty_rejections", worker_count), &worker_count, |b, &worker_count| {
            b.iter(|| black_box(CompletionBenchmark::run(worker_count)));
        });
    }
    group.finish();
}

criterion_group!(benches, router_partition, router_enqueue, api_completion);
criterion_main!(benches);
