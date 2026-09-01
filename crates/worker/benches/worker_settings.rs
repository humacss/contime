use std::mem::size_of;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use contime_worker::{work, ApplyBatch, Checkpoints, EventInsert, Events, RoutedInput, WorkerConfig};
use criterion::measurement::WallTime;
use criterion::{black_box, criterion_group, criterion_main, BatchSize, BenchmarkGroup, Criterion, Throughput};
use crossbeam_channel::{unbounded, Receiver};

struct BenchInput(u128);

#[derive(Default)]
struct BenchEvents(usize);

impl Events<BenchInput> for BenchEvents {
    type Config = ();
    type Rejection = ();
    type Time = u64;

    fn create(_snapshot_id: u128, _config: &(), _horizon: &u64) -> Self {
        Self::default()
    }

    fn insert(&mut self, input: BenchInput) -> EventInsert<()> {
        self.0 = black_box(self.0.wrapping_add(input.0 as usize));
        EventInsert { changed: true, rejections: Vec::new() }
    }

    fn dirty_time(&self) -> &u64 {
        &0
    }

    fn prune_before(&mut self, _horizon: &u64) {}
}

struct BenchCheckpoints;

impl Checkpoints<BenchEvents> for BenchCheckpoints {
    type Config = ();
    type Context = Arc<AtomicUsize>;
    type Time = u64;

    fn create(_snapshot_id: u128, _config: &()) -> Self {
        Self
    }

    fn update(&mut self, _events: &mut BenchEvents, context: &mut Self::Context) {
        context.fetch_add(1, Ordering::Relaxed);
    }

    fn advance_before(&mut self, _events: &BenchEvents, _context: &mut Self::Context, _horizon: &u64) {}
}

type BenchCompletion = crossbeam_channel::Sender<Vec<()>>;

fn config(replays_per_receive: usize) -> WorkerConfig {
    WorkerConfig {
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

trait BenchmarkInput {
    fn input_id(&self) -> u128;
}

impl<const PAYLOAD_BYTES: usize> BenchmarkInput for OwnershipEvent<PAYLOAD_BYTES> {
    fn input_id(&self) -> u128 {
        self.input_id
    }
}

struct SharedInput<I>(Arc<I>);

impl<I> BenchmarkInput for SharedInput<I>
where
    I: BenchmarkInput,
{
    fn input_id(&self) -> u128 {
        self.0.input_id()
    }
}

#[derive(Default)]
struct OwnershipEvents(usize);

impl<I> Events<I> for OwnershipEvents
where
    I: BenchmarkInput,
{
    type Config = ();
    type Rejection = ();
    type Time = u64;

    fn create(_snapshot_id: u128, _config: &(), _horizon: &u64) -> Self {
        Self::default()
    }

    fn insert(&mut self, input: I) -> EventInsert<()> {
        self.0 = black_box(self.0.wrapping_add(input.input_id() as usize));
        EventInsert { changed: true, rejections: Vec::new() }
    }

    fn dirty_time(&self) -> &u64 {
        &0
    }

    fn prune_before(&mut self, _horizon: &u64) {}
}

struct OwnershipCheckpoints;

impl Checkpoints<OwnershipEvents> for OwnershipCheckpoints {
    type Config = ();
    type Context = Arc<AtomicUsize>;
    type Time = u64;

    fn create(_snapshot_id: u128, _config: &()) -> Self {
        Self
    }

    fn update(&mut self, events: &mut OwnershipEvents, context: &mut Self::Context) {
        context.fetch_add(events.0, Ordering::Relaxed);
    }

    fn advance_before(&mut self, _events: &OwnershipEvents, _context: &mut Self::Context, _horizon: &u64) {}
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
    I: BenchmarkInput + 'static,
{
    group.bench_function(ownership, |bencher| {
        bencher.iter_batched(
            || (ownership_batches(make_input), Arc::new(AtomicUsize::new(0))),
            |(receiver, context)| {
                work::<_, OwnershipEvents, OwnershipCheckpoints>(receiver, config(1), (), (), Arc::clone(&context));
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
                        work::<_, BenchEvents, BenchCheckpoints>(receiver, config(replay_budget), (), (), Arc::clone(&context));
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
