use std::hint::black_box;
use std::time::Duration;

use contime_runtime::{Router, Runtime, RuntimeConfig, Worker};
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
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
    Event(PendingEvent),
}

enum WorkerInput {
    Event(PendingEvent),
}

struct PendingEvent {
    event: BenchmarkEvent,
    completion: Sender<()>,
}

struct BenchmarkRouter;

impl Router for BenchmarkRouter {
    type Input = RouterInput;
    type WorkerInput = WorkerInput;
    type Error = ();

    fn run(self, input: Receiver<Self::Input>, workers: Vec<Sender<Self::WorkerInput>>) -> Result<(), Self::Error> {
        for message in input {
            match message {
                RouterInput::Event(pending) => workers[pending.event.worker_index].send(WorkerInput::Event(pending)).unwrap(),
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
                WorkerInput::Event(pending) => {
                    checksum = checksum.wrapping_add(black_box(pending.event.value));
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
    events: Vec<BenchmarkEvent>,
}

struct PreparedRun {
    events: Vec<PendingEvent>,
    completed: Receiver<()>,
}

impl Harness {
    fn new(router_count: usize, worker_count: usize) -> Self {
        let runtime = Runtime::start(RuntimeConfig { router_count, worker_count }, |_| BenchmarkRouter, |_| BenchmarkWorker).unwrap();
        let events = (0..INPUT_COUNT)
            .map(|value| BenchmarkEvent { router_index: value % router_count, worker_index: value % worker_count, value })
            .collect();
        Self { runtime, events }
    }

    fn prepare(&self) -> PreparedRun {
        let (completion, completed) = unbounded();
        let events = self.events.iter().map(|event| PendingEvent { event: *event, completion: completion.clone() }).collect();
        drop(completion);
        PreparedRun { events, completed }
    }

    fn run(&self, prepared: PreparedRun) {
        for pending in prepared.events {
            self.runtime.inputs()[pending.event.router_index].send(RouterInput::Event(pending)).unwrap();
        }
        assert!(prepared.completed.recv().is_err());
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
        harness.run(harness.prepare());
        group.bench_function(BenchmarkId::new("topology", format!("{router_count}_routers_{worker_count}_workers")), |bencher| {
            bencher.iter_batched(|| harness.prepare(), |prepared| harness.run(prepared), BatchSize::SmallInput)
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
