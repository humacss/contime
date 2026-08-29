use std::hint::black_box;
use std::thread::JoinHandle;
use std::time::Duration;

use contime_runtime::{Router, Runtime, RuntimeConfig, ThreadOutcome, Worker};
use criterion::{criterion_group, criterion_main, BenchmarkGroup, BenchmarkId, Criterion, Throughput};
use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use pprof::criterion::{Output, PProfProfiler};

const INPUT_COUNTS: [usize; 5] = [1, 10, 100, 1_000, 10_000];

#[derive(Clone, Copy)]
struct Event {
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
    event: Event,
    completion: Sender<()>,
}

struct RelayRouter;

impl Router for RelayRouter {
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

struct DrainWorker;

impl Worker for DrainWorker {
    type Input = WorkerInput;
    type Error = ();

    fn run(self, input: Receiver<Self::Input>) -> Result<(), Self::Error> {
        drain_worker(input);
        Ok(())
    }
}

fn drain_worker(input: Receiver<WorkerInput>) {
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
}

fn events(input_count: usize) -> Vec<Event> {
    (0..input_count).map(|value| Event { router_index: 0, worker_index: 0, value }).collect()
}

struct LocalChannelHarness {
    input: Sender<RouterInput>,
    output: Receiver<RouterInput>,
    events: Vec<Event>,
}

impl LocalChannelHarness {
    fn new(input_count: usize) -> Self {
        let (input, output) = unbounded();
        Self { input, output, events: events(input_count) }
    }

    fn run(&self) {
        let (completion, completed) = unbounded();
        for event in &self.events {
            let pending = PendingEvent { event: *event, completion: completion.clone() };
            self.input.send(RouterInput::Event(pending)).unwrap();
        }
        drop(completion);
        for _ in &self.events {
            match self.output.recv().unwrap() {
                RouterInput::Event(pending) => {
                    black_box(pending.event);
                    drop(pending.completion);
                }
            }
        }
        assert!(completed.recv().is_err());
    }
}

struct OneHopHarness {
    input: Sender<WorkerInput>,
    worker: JoinHandle<()>,
    events: Vec<Event>,
}

impl OneHopHarness {
    fn new(input_count: usize) -> Self {
        let (input, output) = unbounded();
        Self::from_channel(input_count, input, output)
    }

    fn bounded(input_count: usize) -> Self {
        let (input, output) = bounded(input_count + 1);
        Self::from_channel(input_count, input, output)
    }

    fn from_channel(input_count: usize, input: Sender<WorkerInput>, output: Receiver<WorkerInput>) -> Self {
        let worker = std::thread::spawn(move || drain_worker(output));
        Self { input, worker, events: events(input_count) }
    }

    fn run(&self) {
        let (completion, completed) = unbounded();
        for event in &self.events {
            let pending = PendingEvent { event: *event, completion: completion.clone() };
            self.input.send(WorkerInput::Event(pending)).unwrap();
        }
        drop(completion);
        assert!(completed.recv().is_err());
    }

    fn shutdown(self) {
        drop(self.input);
        self.worker.join().unwrap();
    }
}

struct BoundedTwoHopHarness {
    input: Sender<RouterInput>,
    router: JoinHandle<()>,
    worker: JoinHandle<()>,
    events: Vec<Event>,
}

impl BoundedTwoHopHarness {
    fn new(input_count: usize) -> Self {
        let (input, router_input) = bounded(input_count + 1);
        let (worker_input, worker_output) = bounded(input_count + 1);
        let router = std::thread::spawn(move || {
            for message in router_input {
                match message {
                    RouterInput::Event(pending) => worker_input.send(WorkerInput::Event(pending)).unwrap(),
                }
            }
        });
        let worker = std::thread::spawn(move || drain_worker(worker_output));
        Self { input, router, worker, events: events(input_count) }
    }

    fn run(&self) {
        let (completion, completed) = unbounded();
        for event in &self.events {
            let pending = PendingEvent { event: *event, completion: completion.clone() };
            self.input.send(RouterInput::Event(pending)).unwrap();
        }
        drop(completion);
        assert!(completed.recv().is_err());
    }

    fn shutdown(self) {
        drop(self.input);
        self.router.join().unwrap();
        self.worker.join().unwrap();
    }
}

struct TwoHopHarness {
    runtime: Runtime<RouterInput, (), ()>,
    events: Vec<Event>,
}

impl TwoHopHarness {
    fn new(input_count: usize) -> Self {
        let runtime = Runtime::start(RuntimeConfig { router_count: 1, worker_count: 1 }, |_| RelayRouter, |_| DrainWorker).unwrap();
        Self { runtime, events: events(input_count) }
    }

    fn run(&self) {
        let (completion, completed) = unbounded();
        for event in &self.events {
            let pending = PendingEvent { event: *event, completion: completion.clone() };
            self.runtime.inputs()[event.router_index].send(RouterInput::Event(pending)).unwrap();
        }
        drop(completion);
        assert!(completed.recv().is_err());
    }

    fn shutdown(self) {
        let report = self.runtime.shutdown();
        assert_eq!(report.routers, vec![ThreadOutcome::Completed]);
        assert_eq!(report.workers, vec![ThreadOutcome::Completed]);
    }
}

fn bench_count(group: &mut BenchmarkGroup<'_, criterion::measurement::WallTime>, input_count: usize) {
    group.throughput(Throughput::Elements(input_count as u64));

    let local = LocalChannelHarness::new(input_count);
    group.bench_with_input(BenchmarkId::new("local_send_and_recv", input_count), &input_count, |bencher, _| bencher.iter(|| local.run()));

    let one_hop = OneHopHarness::new(input_count);
    one_hop.run();
    group.bench_with_input(BenchmarkId::new("one_thread_hop", input_count), &input_count, |bencher, _| bencher.iter(|| one_hop.run()));
    one_hop.shutdown();

    let two_hop = TwoHopHarness::new(input_count);
    two_hop.run();
    group.bench_with_input(BenchmarkId::new("router_and_worker_hops", input_count), &input_count, |bencher, _| {
        bencher.iter(|| two_hop.run())
    });
    two_hop.shutdown();

    if input_count >= 1_000 {
        let bounded_one_hop = OneHopHarness::bounded(input_count);
        bounded_one_hop.run();
        group.bench_with_input(BenchmarkId::new("bounded_one_thread_hop", input_count), &input_count, |bencher, _| {
            bencher.iter(|| bounded_one_hop.run())
        });
        bounded_one_hop.shutdown();

        let bounded_two_hop = BoundedTwoHopHarness::new(input_count);
        bounded_two_hop.run();
        group.bench_with_input(BenchmarkId::new("bounded_router_and_worker_hops", input_count), &input_count, |bencher, _| {
            bencher.iter(|| bounded_two_hop.run())
        });
        bounded_two_hop.shutdown();
    }
}

fn benchmark_breakdown(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("breakdown");
    for input_count in INPUT_COUNTS {
        bench_count(&mut group, input_count);
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
        .sample_size(30)
        .with_profiler(PProfProfiler::new(1_000, Output::Flamegraph(None)));
    targets = benchmark_breakdown
}
criterion_main!(benches);
