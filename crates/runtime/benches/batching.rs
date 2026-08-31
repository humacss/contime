use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

use contime_runtime::{Router, Runtime, ThreadOutcome, Worker};
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use crossbeam_channel::{unbounded, Receiver, Sender};
use pprof::criterion::{Output, PProfProfiler};

const WORKLOADS: [(usize, usize); 6] = [(1, 1), (1, 100), (1, 1_000), (10, 1_000), (100, 1_000), (1_000, 1_000)];

#[derive(Clone, Copy)]
struct Event {
    value: usize,
}

#[derive(Clone)]
struct EventBatch {
    worker_index: usize,
    events: Arc<[Event]>,
}

enum RouterInput {
    Batch(PendingBatch),
}

enum WorkerInput {
    Batch(PendingBatch),
}

struct PendingBatch {
    batch: EventBatch,
    completion: Sender<()>,
}

struct BatchRouter;

impl Router for BatchRouter {
    type Input = RouterInput;
    type WorkerInput = WorkerInput;
    type Error = ();

    fn run(self, input: Receiver<Self::Input>, workers: Vec<Sender<Self::WorkerInput>>) -> Result<(), Self::Error> {
        for message in input {
            match message {
                RouterInput::Batch(pending) => workers[pending.batch.worker_index].send(WorkerInput::Batch(pending)).unwrap(),
            }
        }
        Ok(())
    }
}

struct BatchWorker;

impl Worker for BatchWorker {
    type Input = WorkerInput;
    type Error = ();

    fn run(self, input: Receiver<Self::Input>) -> Result<(), Self::Error> {
        let mut checksum = 0usize;
        for message in input {
            match message {
                WorkerInput::Batch(pending) => {
                    for event in pending.batch.events.iter() {
                        checksum = checksum.wrapping_add(black_box(event.value));
                    }
                    black_box(checksum);
                    drop(pending.completion);
                }
            }
        }
        black_box(checksum);
        Ok(())
    }
}

struct Harness {
    runtime: Runtime<RouterInput, (), ()>,
    batches: Vec<EventBatch>,
}

struct PreparedRun {
    batches: Vec<PendingBatch>,
    completed: Receiver<()>,
}

impl Harness {
    fn new(router_count: usize, worker_count: usize, batches_per_worker: usize, events_per_batch: usize) -> Self {
        let runtime =
            Runtime::start((0..router_count).map(|_| BatchRouter).collect(), (0..worker_count).map(|_| BatchWorker).collect()).unwrap();
        let batches = (0..worker_count)
            .flat_map(|worker_index| {
                (0..batches_per_worker).map(move |batch_index| {
                    let first_value = (worker_index * batches_per_worker + batch_index) * events_per_batch;
                    let events = (first_value..first_value + events_per_batch).map(|value| Event { value }).collect::<Vec<_>>().into();
                    EventBatch { worker_index, events }
                })
            })
            .collect();
        Self { runtime, batches }
    }

    fn prepare(&self) -> PreparedRun {
        let (completion, completed) = unbounded();
        let batches = self.batches.iter().map(|batch| PendingBatch { batch: batch.clone(), completion: completion.clone() }).collect();
        drop(completion);
        PreparedRun { batches, completed }
    }

    fn run(&self, prepared: PreparedRun) {
        for pending in prepared.batches {
            self.runtime.input().send(RouterInput::Batch(pending)).unwrap();
        }
        assert!(prepared.completed.recv().is_err());
    }

    fn shutdown(self) {
        let report = self.runtime.shutdown();
        assert!(report.routers.iter().all(|outcome| *outcome == ThreadOutcome::Completed));
        assert!(report.workers.iter().all(|outcome| *outcome == ThreadOutcome::Completed));
    }
}

fn process_directly(batches: &[EventBatch]) {
    let mut checksum = 0usize;
    for batch in batches {
        for event in batch.events.iter() {
            checksum = checksum.wrapping_add(black_box(event.value));
        }
    }
    black_box(checksum);
}

fn benchmark_batch_sizes(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("batching/realistic_batches");

    for (router_count, worker_count) in [(1, 1), (2, 4)] {
        for (batches_per_worker, events_per_batch) in WORKLOADS {
            let events_per_worker = batches_per_worker * events_per_batch;
            let total_events = events_per_worker * worker_count;
            group.throughput(Throughput::Elements(total_events as u64));
            let harness = Harness::new(router_count, worker_count, batches_per_worker, events_per_batch);
            harness.run(harness.prepare());
            if router_count == 1 {
                group.bench_function(
                    BenchmarkId::new("direct_no_channels", format!("{batches_per_worker}_batches_{events_per_batch}_events_each")),
                    |bencher| bencher.iter(|| process_directly(&harness.batches)),
                );
            }
            group.bench_function(
                BenchmarkId::new(
                    format!("{router_count}_routers_{worker_count}_workers"),
                    format!(
                        "{batches_per_worker}_batches_per_worker_{events_per_batch}_events_per_batch_{events_per_worker}_events_per_worker"
                    ),
                ),
                |bencher| bencher.iter_batched(|| harness.prepare(), |prepared| harness.run(prepared), BatchSize::SmallInput),
            );
            harness.shutdown();
        }
    }

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(2))
        .measurement_time(Duration::from_secs(5))
        .sample_size(50)
        .with_profiler(PProfProfiler::new(1_000, Output::Flamegraph(None)));
    targets = benchmark_batch_sizes
}
criterion_main!(benches);
