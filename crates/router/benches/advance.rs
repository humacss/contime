use contime_router::{route_advance, AdvanceInput, AdvanceWorkerOutput};
use criterion::{black_box, criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use crossbeam_channel::{unbounded, Receiver, Sender};

struct Advance {
    time: u64,
    completion: Sender<()>,
}

impl AdvanceInput for Advance {
    type Time = u64;
    type Completion = Sender<()>;

    fn into_parts(self) -> (u64, Sender<()>) {
        (self.time, self.completion)
    }
}

struct WorkerAdvance {
    time: u64,
    completion: Sender<()>,
}

impl AdvanceWorkerOutput<u64, Sender<()>> for WorkerAdvance {
    fn advance(time: u64, completion: Sender<()>) -> Self {
        Self { time, completion }
    }
}

fn channels(worker_count: usize) -> (Vec<Sender<WorkerAdvance>>, Vec<Receiver<WorkerAdvance>>) {
    (0..worker_count).map(|_| unbounded()).unzip()
}

fn benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("router/advance");
    for worker_count in [1_usize, 4, 16] {
        group.throughput(Throughput::Elements(worker_count as u64));
        group.bench_with_input(BenchmarkId::new("workers", worker_count), &worker_count, |bencher, worker_count| {
            bencher.iter_batched(
                || {
                    let (workers, outputs) = channels(*worker_count);
                    let (completion, done) = unbounded();
                    (workers, outputs, completion, done)
                },
                |(workers, outputs, completion, done)| {
                    route_advance(Advance { time: black_box(50), completion }, &workers).unwrap();
                    drop(workers);
                    for output in outputs {
                        let message = output.recv().unwrap();
                        black_box(message.time);
                        drop(message.completion);
                    }
                    black_box(done.into_iter().count());
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group!(benches, benchmarks);
criterion_main!(benches);
