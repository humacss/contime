use std::hint::black_box;

use contime_router::{
    route_event_query, route_snapshot_query, EventQueryInput, EventQueryWorkerOutput, SnapshotQueryInput, SnapshotQueryWorkerOutput,
};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use crossbeam_channel::{unbounded, Receiver, Sender};

#[derive(Clone)]
struct Response;

struct SnapshotQuery {
    ids: Vec<u128>,
}

impl SnapshotQueryInput for SnapshotQuery {
    type Time = u64;
    type Response = Response;

    fn into_parts(self) -> (Self::Time, Vec<u128>, Self::Response) {
        (42, self.ids, Response)
    }
}

struct EventQuery;

impl EventQueryInput for EventQuery {
    type Time = u64;
    type Response = Response;

    fn into_parts(self) -> (u128, Self::Time, Self::Time, Self::Response) {
        (7, 10, 20, Response)
    }
}

struct WorkerMessage;

impl SnapshotQueryWorkerOutput<u64, Response> for WorkerMessage {
    fn snapshot_query(_time: u64, _snapshot_ids: Vec<u128>, _response: Response) -> Self {
        Self
    }
}

impl EventQueryWorkerOutput<u64, Response> for WorkerMessage {
    fn event_query(_snapshot_id: u128, _from: u64, _to: u64, _response: Response) -> Self {
        Self
    }
}

fn workers(count: usize) -> (Vec<Sender<WorkerMessage>>, Vec<Receiver<WorkerMessage>>) {
    (0..count).map(|_| unbounded()).unzip()
}

fn query_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("router/query");

    for worker_count in [1_usize, 2, 4, 8, 10] {
        for query_count in [1_usize, 10, 100, 1_000] {
            group.bench_with_input(
                BenchmarkId::new(format!("snapshots/{worker_count}_workers"), query_count),
                &query_count,
                |bencher, &query_count| {
                    bencher.iter_batched(
                        || workers(worker_count),
                        |(senders, receivers)| {
                            route_snapshot_query(7, SnapshotQuery { ids: (0..query_count as u128).collect() }, &senders).unwrap();
                            black_box(receivers)
                        },
                        criterion::BatchSize::SmallInput,
                    );
                },
            );
        }

        group.bench_with_input(BenchmarkId::new("event", worker_count), &worker_count, |bencher, &worker_count| {
            bencher.iter_batched(
                || workers(worker_count),
                |(senders, receivers)| {
                    route_event_query(7, EventQuery, &senders).unwrap();
                    black_box(receivers)
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

criterion_group!(benches, query_benchmarks);
criterion_main!(benches);
