use std::cell::Cell;
use std::mem::size_of;
use std::rc::Rc;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Arc;

use contime_memory::{ConservativeTrackedSize, SizeDelta, TrackedArc, TrackedBox, TrackedMemoryBudget, TrackedSizeDelta};
use criterion::{criterion_group, criterion_main, measurement::WallTime, BenchmarkGroup, BenchmarkId, Criterion, Throughput};

const OPERATIONS: u64 = 1_000;

trait BudgetFixture: TrackedMemoryBudget + Default + 'static {
    const NAME: &'static str;
}

#[derive(Clone, Copy, Default)]
struct NoopBudget;

impl TrackedMemoryBudget for NoopBudget {
    fn apply_delta(&self, delta: SizeDelta) {
        std::hint::black_box(delta);
    }

    fn has_buffer(&self) -> bool {
        true
    }

    fn buffer_size(&self) -> usize {
        usize::MAX
    }
}

impl BudgetFixture for NoopBudget {
    const NAME: &'static str = "noop";
}

#[derive(Clone, Default)]
struct LocalBudget(Rc<Cell<isize>>);

impl TrackedMemoryBudget for LocalBudget {
    fn apply_delta(&self, delta: SizeDelta) {
        let change = signed(delta);
        self.0.set(self.0.get().saturating_add(change));
    }

    fn has_buffer(&self) -> bool {
        true
    }

    fn buffer_size(&self) -> usize {
        usize::MAX
    }
}

impl BudgetFixture for LocalBudget {
    const NAME: &'static str = "local";
}

#[derive(Clone, Default)]
struct AtomicBudget(Arc<AtomicIsize>);

impl TrackedMemoryBudget for AtomicBudget {
    fn apply_delta(&self, delta: SizeDelta) {
        self.0.fetch_add(signed(delta), Ordering::Relaxed);
    }

    fn has_buffer(&self) -> bool {
        true
    }

    fn buffer_size(&self) -> usize {
        usize::MAX
    }
}

impl BudgetFixture for AtomicBudget {
    const NAME: &'static str = "atomic";
}

fn signed(delta: SizeDelta) -> isize {
    match delta {
        SizeDelta::Increase(bytes) => bytes as isize,
        SizeDelta::Decrease(bytes) => -(bytes as isize),
        SizeDelta::Unchanged => 0,
    }
}

struct Event([u8; 64]);

impl ConservativeTrackedSize for Event {
    fn conservative_tracked_size(&self) -> usize {
        std::hint::black_box(&self.0);
        size_of::<Self>()
    }
}

#[derive(Clone)]
struct Payload(Vec<u8>);

impl Payload {
    fn new(bytes: usize) -> Self {
        Self(vec![0; bytes])
    }
}

impl ConservativeTrackedSize for Payload {
    fn conservative_tracked_size(&self) -> usize {
        size_of::<Self>().saturating_add(self.0.capacity())
    }
}

impl TrackedSizeDelta for Payload {
    fn size_delta<R>(&mut self, action: impl FnOnce(&mut Self) -> R) -> (R, SizeDelta) {
        let before = self.conservative_tracked_size();
        let result = action(self);
        let after = self.conservative_tracked_size();
        let delta = match after.cmp(&before) {
            std::cmp::Ordering::Greater => SizeDelta::Increase(after - before),
            std::cmp::Ordering::Less => SizeDelta::Decrease(before - after),
            std::cmp::Ordering::Equal => SizeDelta::Unchanged,
        };
        (result, delta)
    }
}

fn standard_arc_create_drop(group: &mut BenchmarkGroup<'_, WallTime>) {
    group.bench_function(BenchmarkId::from_parameter("standard"), |bencher| {
        bencher.iter(|| {
            for index in 0..OPERATIONS {
                drop(std::hint::black_box(Arc::new(Event([index as u8; 64]))));
            }
        });
    });
}

fn standard_arc_clone_drop(group: &mut BenchmarkGroup<'_, WallTime>) {
    let original = Arc::new(Event([7; 64]));
    group.bench_function(BenchmarkId::from_parameter("standard"), |bencher| {
        bencher.iter(|| {
            for _ in 0..OPERATIONS {
                drop(std::hint::black_box(original.clone()));
            }
        });
    });
}

fn standard_box_create_drop(group: &mut BenchmarkGroup<'_, WallTime>) {
    group.bench_function(BenchmarkId::from_parameter("standard"), |bencher| {
        bencher.iter(|| {
            for _ in 0..OPERATIONS {
                drop(std::hint::black_box(Box::new(Payload::new(64))));
            }
        });
    });
}

fn standard_box_update(group: &mut BenchmarkGroup<'_, WallTime>) {
    let mut snapshots = (0..OPERATIONS).map(|_| Box::new(Payload::new(64))).collect::<Vec<_>>();
    group.bench_function(BenchmarkId::from_parameter("standard"), |bencher| {
        bencher.iter(|| {
            for snapshot in &mut snapshots {
                snapshot.0[0] = std::hint::black_box(snapshot.0[0] ^ 1);
            }
        });
    });
}

fn standard_box_deep_clone_drop(group: &mut BenchmarkGroup<'_, WallTime>, payload_bytes: usize) {
    let original = Box::new(Payload::new(payload_bytes));
    group.bench_with_input(BenchmarkId::new("standard", payload_bytes), &payload_bytes, |bencher, _| {
        bencher.iter(|| {
            for _ in 0..OPERATIONS {
                drop(std::hint::black_box(original.clone()));
            }
        });
    });
}

fn arc_create_drop<B>(group: &mut BenchmarkGroup<'_, WallTime>)
where
    B: BudgetFixture,
{
    let budget = B::default();
    group.bench_function(BenchmarkId::from_parameter(B::NAME), |bencher| {
        bencher.iter(|| {
            for index in 0..OPERATIONS {
                drop(std::hint::black_box(TrackedArc::new(Event([index as u8; 64]), budget.clone())));
            }
        });
    });
}

fn arc_clone_drop<B>(group: &mut BenchmarkGroup<'_, WallTime>)
where
    B: BudgetFixture,
{
    let original = TrackedArc::new(Event([7; 64]), B::default());
    group.bench_function(BenchmarkId::from_parameter(B::NAME), |bencher| {
        bencher.iter(|| {
            for _ in 0..OPERATIONS {
                drop(std::hint::black_box(original.clone()));
            }
        });
    });
}

fn box_create_drop<B>(group: &mut BenchmarkGroup<'_, WallTime>)
where
    B: BudgetFixture,
{
    let budget = B::default();
    group.bench_function(BenchmarkId::from_parameter(B::NAME), |bencher| {
        bencher.iter(|| {
            for _ in 0..OPERATIONS {
                drop(std::hint::black_box(TrackedBox::new(Payload::new(64), budget.clone())));
            }
        });
    });
}

fn box_update<B>(group: &mut BenchmarkGroup<'_, WallTime>)
where
    B: BudgetFixture,
{
    let budget = B::default();
    let mut snapshots = (0..OPERATIONS).map(|_| TrackedBox::new(Payload::new(64), budget.clone())).collect::<Vec<_>>();
    group.bench_function(BenchmarkId::from_parameter(B::NAME), |bencher| {
        bencher.iter(|| {
            for snapshot in &mut snapshots {
                snapshot.update(|payload| {
                    payload.0[0] = std::hint::black_box(payload.0[0] ^ 1);
                });
            }
        });
    });
}

fn box_deep_clone_drop<B>(group: &mut BenchmarkGroup<'_, WallTime>, payload_bytes: usize)
where
    B: BudgetFixture,
{
    let original = TrackedBox::new(Payload::new(payload_bytes), B::default());
    group.bench_with_input(BenchmarkId::new(B::NAME, payload_bytes), &payload_bytes, |bencher, _| {
        bencher.iter(|| {
            for _ in 0..OPERATIONS {
                drop(std::hint::black_box(original.clone()));
            }
        });
    });
}

fn ownership(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("memory/ownership/arc_create_drop");
    group.throughput(Throughput::Elements(OPERATIONS));
    standard_arc_create_drop(&mut group);
    arc_create_drop::<NoopBudget>(&mut group);
    arc_create_drop::<LocalBudget>(&mut group);
    arc_create_drop::<AtomicBudget>(&mut group);
    group.finish();

    let mut group = criterion.benchmark_group("memory/ownership/arc_clone_drop");
    group.throughput(Throughput::Elements(OPERATIONS));
    standard_arc_clone_drop(&mut group);
    arc_clone_drop::<NoopBudget>(&mut group);
    arc_clone_drop::<LocalBudget>(&mut group);
    arc_clone_drop::<AtomicBudget>(&mut group);
    group.finish();

    let mut group = criterion.benchmark_group("memory/ownership/box_create_drop_64");
    group.throughput(Throughput::Elements(OPERATIONS));
    standard_box_create_drop(&mut group);
    box_create_drop::<NoopBudget>(&mut group);
    box_create_drop::<LocalBudget>(&mut group);
    box_create_drop::<AtomicBudget>(&mut group);
    group.finish();

    let mut group = criterion.benchmark_group("memory/ownership/box_update_64");
    group.throughput(Throughput::Elements(OPERATIONS));
    standard_box_update(&mut group);
    box_update::<NoopBudget>(&mut group);
    box_update::<LocalBudget>(&mut group);
    box_update::<AtomicBudget>(&mut group);
    group.finish();

    let mut group = criterion.benchmark_group("memory/ownership/box_deep_clone_drop");
    group.throughput(Throughput::Elements(OPERATIONS));
    for payload_bytes in [64, 256, 1_024] {
        standard_box_deep_clone_drop(&mut group, payload_bytes);
        box_deep_clone_drop::<NoopBudget>(&mut group, payload_bytes);
        box_deep_clone_drop::<LocalBudget>(&mut group, payload_bytes);
        box_deep_clone_drop::<AtomicBudget>(&mut group, payload_bytes);
    }
    group.finish();
}

criterion_group!(benches, ownership);
criterion_main!(benches);
