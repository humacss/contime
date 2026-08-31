use std::mem::size_of;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use contime_worker::{
    work, ApplyBatch, CheckpointResult, Checkpoints, CheckpointsCreated, EventInsert, Events, EventsCreated, RoutedInput, WorkerConfig,
    WorkerInput, WorkerRejection,
};
use criterion::measurement::WallTime;
use criterion::{black_box, criterion_group, criterion_main, BatchSize, BenchmarkGroup, Criterion, Throughput};
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

    fn insert(&mut self, _input: BenchInput, _limit: u64) -> EventInsert<()> {
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
                input: BenchInput((batch_index * snapshot_count + snapshot_index) as u128),
            })
            .collect();
        sender.send(ApplyBatch { inputs, completion }).unwrap();
    }
    drop(sender);
    receiver
}

#[repr(C)]
struct OwnershipEvent<const PAYLOAD_BYTES: usize> {
    input_id: u128,
    payload: [u8; PAYLOAD_BYTES],
}

impl<const PAYLOAD_BYTES: usize> WorkerInput for OwnershipEvent<PAYLOAD_BYTES> {
    fn input_id(&self) -> u128 {
        self.input_id
    }

    fn conservative_size(&self) -> u64 {
        size_of::<Self>() as u64
    }
}

struct SharedInput<I>(Arc<I>);

impl<I> WorkerInput for SharedInput<I>
where
    I: WorkerInput,
{
    fn input_id(&self) -> u128 {
        self.0.input_id()
    }

    fn conservative_size(&self) -> u64 {
        self.0.conservative_size()
    }
}

#[derive(Default)]
struct OwnershipEvents(usize);

impl<I> Events<I> for OwnershipEvents
where
    I: WorkerInput,
{
    type Config = ();
    type Rejection = ();

    fn create(_snapshot_id: u128, _config: &(), _limit: u64) -> Option<EventsCreated<Self>> {
        Some(EventsCreated { events: Self::default(), retained_bytes_delta: 0 })
    }

    fn insert(&mut self, input: I, _limit: u64) -> EventInsert<()> {
        self.0 = black_box(self.0.wrapping_add(input.input_id() as usize));
        EventInsert { retained_bytes_delta: input.conservative_size() as i64, changed: true, rejections: Vec::new() }
    }
}

struct OwnershipCheckpoints;

impl Checkpoints<OwnershipEvents> for OwnershipCheckpoints {
    type Config = ();
    type Context = Arc<AtomicUsize>;

    fn create(_snapshot_id: u128, _config: &(), _limit: u64) -> CheckpointsCreated<Self> {
        CheckpointsCreated { checkpoints: Self, retained_bytes_delta: 0 }
    }

    fn update(&mut self, events: &OwnershipEvents, context: &mut Self::Context, _limit: u64) -> CheckpointResult {
        context.fetch_add(events.0, Ordering::Relaxed);
        CheckpointResult { retained_bytes_delta: 0 }
    }
}

const _: () = assert!(size_of::<OwnershipEvent<48>>() == 64);
const _: () = assert!(size_of::<OwnershipEvent<192>>() == 208);
const _: () = assert!(size_of::<OwnershipEvent<992>>() == 1_008);
const _: () = assert!(size_of::<SharedInput<OwnershipEvent<48>>>() == size_of::<Arc<OwnershipEvent<48>>>());

fn ownership_batches<I>(make_input: impl Fn(usize) -> I) -> Receiver<ApplyBatch<I, BenchCompletion>> {
    let (sender, receiver) = unbounded();
    for input_index in 0..1_000 {
        let (completion, _responses) = unbounded();
        let inputs = vec![RoutedInput { snapshot_id: (input_index % 4) as u128, input: make_input(input_index) }];
        sender.send(ApplyBatch { inputs, completion }).unwrap();
    }
    drop(sender);
    receiver
}

fn benchmark_ownership<I>(group: &mut BenchmarkGroup<'_, WallTime>, ownership: &str, make_input: impl Fn(usize) -> I + Copy + 'static)
where
    I: WorkerInput + 'static,
{
    group.bench_function(ownership, |bencher| {
        bencher.iter_batched(
            || (ownership_batches(make_input), Arc::new(AtomicUsize::new(0))),
            |(receiver, context)| {
                work::<I, OwnershipEvents, OwnershipCheckpoints, _>(receiver, config(1), (), (), Arc::clone(&context));
                black_box(context.load(Ordering::Relaxed));
            },
            BatchSize::LargeInput,
        );
    });
}

fn benchmark_ownership_for_size<const PAYLOAD_BYTES: usize>(criterion: &mut Criterion, event_bytes: usize) {
    let mut group = criterion.benchmark_group(format!("worker/ownership/1000_inputs/{event_bytes}_byte_events"));
    group.throughput(Throughput::Elements(1_000));
    benchmark_ownership(&mut group, "owned", |input_index| OwnershipEvent {
        input_id: input_index as u128,
        payload: [input_index as u8; PAYLOAD_BYTES],
    });
    benchmark_ownership(&mut group, "shared", |input_index| {
        SharedInput(Arc::new(OwnershipEvent { input_id: input_index as u128, payload: [input_index as u8; PAYLOAD_BYTES] }))
    });
    group.finish();
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

    benchmark_ownership_for_size::<48>(criterion, 64);
    benchmark_ownership_for_size::<192>(criterion, 208);
    benchmark_ownership_for_size::<992>(criterion, 1_008);
}

criterion_group!(benches, benchmark_work_settings);
criterion_main!(benches);
