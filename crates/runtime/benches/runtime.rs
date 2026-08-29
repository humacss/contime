use std::hint::black_box;
use std::time::Duration;

use contime_runtime::{Router, Runtime, RuntimeConfig, Worker};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use crossbeam_channel::{unbounded, Receiver, Sender};
use pprof::criterion::{Output, PProfProfiler};

const INPUT_COUNT: usize = 1_000;

#[derive(Clone, Copy)]
struct BenchmarkEvent {
    router_index: usize,
    worker_index: usize,
    value: usize,
}

enum RouterInput {
    Event(BenchmarkEvent),
    FlushRouter(Sender<()>),
    FlushWorkers(Sender<()>),
}

enum WorkerInput {
    Event(BenchmarkEvent),
    Flush(Sender<()>),
}

struct BenchmarkRouter;

impl Router for BenchmarkRouter {
    type Input = RouterInput;
    type WorkerInput = WorkerInput;
    type Error = ();

    fn run(self, input: Receiver<Self::Input>, workers: Vec<Sender<Self::WorkerInput>>) -> Result<(), Self::Error> {
        for message in input {
            match message {
                RouterInput::Event(event) => workers[event.worker_index].send(WorkerInput::Event(event)).unwrap(),
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

struct BenchmarkWorker;

impl Worker for BenchmarkWorker {
    type Input = WorkerInput;
    type Error = ();

    fn run(self, input: Receiver<Self::Input>) -> Result<(), Self::Error> {
        let mut checksum = 0usize;
        for message in input {
            match message {
                WorkerInput::Event(event) => checksum = checksum.wrapping_add(black_box(event.value)),
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
    events: Vec<BenchmarkEvent>,
    router_acknowledgements: Receiver<()>,
    router_acknowledger: Sender<()>,
    worker_acknowledgements: Receiver<()>,
    worker_acknowledger: Sender<()>,
    router_count: usize,
    worker_count: usize,
}

impl Harness {
    fn new(router_count: usize, worker_count: usize) -> Self {
        let runtime = Runtime::start(RuntimeConfig { router_count, worker_count }, |_| BenchmarkRouter, |_| BenchmarkWorker).unwrap();
        let events = (0..INPUT_COUNT)
            .map(|value| BenchmarkEvent { router_index: value % router_count, worker_index: value % worker_count, value })
            .collect();
        let (router_acknowledger, router_acknowledgements) = unbounded();
        let (worker_acknowledger, worker_acknowledgements) = unbounded();
        Self {
            runtime,
            events,
            router_acknowledgements,
            router_acknowledger,
            worker_acknowledgements,
            worker_acknowledger,
            router_count,
            worker_count,
        }
    }

    fn run(&self) {
        for event in &self.events {
            self.runtime.inputs()[event.router_index].send(RouterInput::Event(*event)).unwrap();
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
        assert!(report.routers.iter().all(|outcome| *outcome == contime_runtime::ThreadOutcome::Completed));
        assert!(report.workers.iter().all(|outcome| *outcome == contime_runtime::ThreadOutcome::Completed));
    }
}

fn benchmark_runtime(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("runtime/1000_inputs");
    group.throughput(Throughput::Elements(INPUT_COUNT as u64));

    for (router_count, worker_count) in [(1, 1), (2, 4)] {
        let harness = Harness::new(router_count, worker_count);
        harness.run();
        group.bench_function(BenchmarkId::new("topology", format!("{router_count}_routers_{worker_count}_workers")), |bencher| {
            bencher.iter(|| harness.run())
        });
        harness.shutdown();
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
    targets = benchmark_runtime
}
criterion_main!(benches);
