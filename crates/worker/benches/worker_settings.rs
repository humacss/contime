use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use contime_worker::{
    work, ApplyBatch, CheckpointResult, Checkpoints, CheckpointsCreated, EventInsert, Events, EventsCreated, RoutedInput, WorkerConfig,
    WorkerInput, WorkerRejection,
};
use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use crossbeam_channel::{unbounded, Receiver};

struct BenchInput(u128);

impl WorkerInput for BenchInput {
    fn input_id(&self) -> u128 {
        self.0
    }

    fn conservative_size(&self) -> u64 {
        32
    }
}

#[derive(Default)]
struct BenchEvents(usize);

impl Events<BenchInput> for BenchEvents {
    type Config = ();
    type Rejection = ();

    fn create(_snapshot_id: u128, _config: &(), _limit: u64) -> Option<EventsCreated<Self>> {
        Some(EventsCreated { events: Self::default(), retained_bytes_delta: 0 })
    }

    fn insert(&mut self, _input: Arc<BenchInput>, _limit: u64) -> EventInsert<()> {
        self.0 += 1;
        EventInsert { retained_bytes_delta: 32, changed: true, rejections: Vec::new() }
    }
}

struct BenchCheckpoints;

impl Checkpoints<BenchEvents> for BenchCheckpoints {
    type Config = ();
    type Context = Arc<AtomicUsize>;

    fn create(_snapshot_id: u128, _config: &(), _limit: u64) -> CheckpointsCreated<Self> {
        CheckpointsCreated { checkpoints: Self, retained_bytes_delta: 0 }
    }

    fn update(&mut self, _events: &BenchEvents, context: &mut Self::Context, _limit: u64) -> CheckpointResult {
        context.fetch_add(1, Ordering::Relaxed);
        CheckpointResult { retained_bytes_delta: 0 }
    }
}

type BenchCompletion = crossbeam_channel::Sender<Vec<WorkerRejection<()>>>;

fn config(replays_per_receive: usize) -> WorkerConfig {
    WorkerConfig {
        memory_limit: 100_000_000,
        maximum_dirty_age: Duration::from_secs(60),
        replays_per_receive,
        deadline_compaction_minimum: 1_024,
        deadline_compaction_multiplier: 2,
    }
}

fn input_batches(batch_count: usize, snapshot_count: usize) -> Receiver<ApplyBatch<BenchInput, BenchCompletion>> {
    let (sender, receiver) = unbounded();
    for batch_index in 0..batch_count {
        let (completion, _responses) = unbounded();
        let inputs = (0..snapshot_count)
            .map(|snapshot_index| RoutedInput {
                snapshot_id: snapshot_index as u128,
                input: Arc::new(BenchInput((batch_index * snapshot_count + snapshot_index) as u128)),
            })
            .collect();
        sender.send(ApplyBatch { inputs, completion }).unwrap();
    }
    drop(sender);
    receiver
}

fn benchmark_work_settings(criterion: &mut Criterion) {
    for snapshot_count in [1, 4] {
        let mut group = criterion.benchmark_group(format!("worker/settings/1000_batches/{snapshot_count}_snapshots"));
        for replay_budget in [0, 1, 4, 16] {
            group.bench_function(format!("{replay_budget}_replays_per_receive"), |bencher| {
                bencher.iter_batched(
                    || (input_batches(1_000, snapshot_count), Arc::new(AtomicUsize::new(0))),
                    |(receiver, context)| {
                        work::<BenchInput, BenchEvents, BenchCheckpoints, _>(receiver, config(replay_budget), (), (), Arc::clone(&context));
                        black_box(context.load(Ordering::Relaxed));
                    },
                    BatchSize::LargeInput,
                );
            });
        }
        group.finish();
    }
}

criterion_group!(benches, benchmark_work_settings);
criterion_main!(benches);
