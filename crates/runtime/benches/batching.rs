use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

use contime_runtime::{Router, Runtime, RuntimeConfig, ThreadOutcome, Worker};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use crossbeam_channel::{unbounded, Receiver, Sender};
use pprof::criterion::{Output, PProfProfiler};

const EVENT_COUNT: usize = 100_000;
const BATCH_SIZES: [usize; 3] = [1, 100, 1_000];

#[derive(Clone, Copy)]
struct Event {
    value: usize,
}

#[derive(Clone)]
struct EventBatch {
    router_index: usize,
    worker_index: usize,
    events: Arc<[Event]>,
}

enum RouterInput {
    Batch(EventBatch),
    FlushRouter(Sender<()>),
    FlushWorkers(Sender<()>),
}

enum WorkerInput {
    Batch(EventBatch),
    Flush(Sender<()>),
}

struct BatchRouter;

impl Router for BatchRouter {
    type Input = RouterInput;
    type WorkerInput = WorkerInput;
    type Error = ();

    fn run(self, input: Receiver<Self::Input>, workers: Vec<Sender<Self::WorkerInput>>) -> Result<(), Self::Error> {
        for message in input {
            match message {
                RouterInput::Batch(batch) => workers[batch.worker_index].send(WorkerInput::Batch(batch)).unwrap(),
                RouterInput::FlushRouter(sender) => sender.send(()).unwrap(),
                RouterInput::FlushWorkers(sender) => {
                    for worker in &workers {
                        worker.send(WorkerInput::Flush(sender.clone())).unwrap();
                    }
                }
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
                WorkerInput::Batch(batch) => {
                    for event in batch.events.iter() {
                        checksum = checksum.wrapping_add(black_box(event.value));
                    }
                }
                WorkerInput::Flush(sender) => {
                    black_box(checksum);
                    sender.send(()).unwrap();
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
    router_acknowledgements: Receiver<()>,
    router_acknowledger: Sender<()>,
    worker_acknowledgements: Receiver<()>,
    worker_acknowledger: Sender<()>,
    router_count: usize,
    worker_count: usize,
}

impl Harness {
    fn new(router_count: usize, worker_count: usize, batch_size: usize) -> Self {
        assert_eq!(EVENT_COUNT % batch_size, 0);
        let runtime = Runtime::start(RuntimeConfig { router_count, worker_count }, |_| BatchRouter, |_| BatchWorker).unwrap();
        let batches = (0..EVENT_COUNT / batch_size)
            .map(|batch_index| {
                let first_value = batch_index * batch_size;
                let events = (first_value..first_value + batch_size).map(|value| Event { value }).collect::<Vec<_>>().into();
                EventBatch { router_index: batch_index % router_count, worker_index: batch_index % worker_count, events }
            })
            .collect();
        let (router_acknowledger, router_acknowledgements) = unbounded();
        let (worker_acknowledger, worker_acknowledgements) = unbounded();
        Self {
            runtime,
            batches,
            router_acknowledgements,
            router_acknowledger,
            worker_acknowledgements,
            worker_acknowledger,
            router_count,
            worker_count,
        }
    }

    fn run(&self) {
        for batch in &self.batches {
            self.runtime.inputs()[batch.router_index].send(RouterInput::Batch(batch.clone())).unwrap();
        }

        for input in self.runtime.inputs() {
            input.send(RouterInput::FlushRouter(self.router_acknowledger.clone())).unwrap();
        }
        for _ in 0..self.router_count {
            self.router_acknowledgements.recv().unwrap();
        }

        self.runtime.inputs()[0].send(RouterInput::FlushWorkers(self.worker_acknowledger.clone())).unwrap();
        for _ in 0..self.worker_count {
            self.worker_acknowledgements.recv().unwrap();
        }
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
        let batch = batch.clone();
        for event in batch.events.iter() {
            checksum = checksum.wrapping_add(black_box(event.value));
        }
    }
    black_box(checksum);
}

fn benchmark_batch_sizes(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("batching/100000_events");
    group.throughput(Throughput::Elements(EVENT_COUNT as u64));

    for (router_count, worker_count) in [(1, 1), (2, 4)] {
        for batch_size in BATCH_SIZES {
            let harness = Harness::new(router_count, worker_count, batch_size);
            harness.run();
            if router_count == 1 {
                group.bench_function(BenchmarkId::new("direct_no_channels", format!("{batch_size}_events_per_batch")), |bencher| {
                    bencher.iter(|| process_directly(&harness.batches))
                });
            }
            group.bench_function(
                BenchmarkId::new(format!("{router_count}_routers_{worker_count}_workers"), format!("{batch_size}_events_per_batch")),
                |bencher| bencher.iter(|| harness.run()),
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
