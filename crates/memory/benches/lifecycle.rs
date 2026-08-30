use std::mem::size_of;
use std::sync::mpsc;

use contime_memory::{AtomicMemoryBudget, CachedAccount, ConservativeTrackedSize, MemoryBudgetConfig, TrackedArc, TrackedBox};
use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};

struct Event([u8; 64]);

impl ConservativeTrackedSize for Event {
    fn conservative_tracked_size(&self) -> usize {
        size_of::<Self>()
    }
}

struct Message(TrackedArc<Event>);

impl ConservativeTrackedSize for Message {
    fn conservative_tracked_size(&self) -> usize {
        0
    }
}

#[derive(Clone)]
struct Snapshot(Vec<usize>);

impl ConservativeTrackedSize for Snapshot {
    fn conservative_tracked_size(&self) -> usize {
        size_of::<Self>() + self.0.capacity() * size_of::<usize>()
    }
}

fn budget() -> AtomicMemoryBudget {
    AtomicMemoryBudget::new(MemoryBudgetConfig { hard_limit: usize::MAX, concurrent_actions: 0, action_buffer: usize::MAX }).unwrap()
}

fn lifecycle(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("memory/lifecycle");
    group.throughput(Throughput::Elements(1_000));

    let message_budget = budget();
    let events = (0..1_000).map(|index| TrackedArc::new(Event([index as u8; 64]), message_budget.clone())).collect::<Vec<_>>();
    let (sender, receiver) = mpsc::channel();
    group.bench_function("messages/1000", |bencher| {
        bencher.iter(|| {
            for event in &events {
                sender.send(TrackedBox::new(Message(event.clone()), message_budget.clone())).unwrap();
            }
            for _ in 0..1_000 {
                let message = receiver.recv().unwrap();
                std::hint::black_box(message.0 .0[0]);
            }
        });
    });

    let measured = TrackedBox::new(Snapshot(vec![0; 1_000]), budget());
    group.bench_function("snapshots/measured_updates/1000", |bencher| {
        bencher.iter_batched(
            || measured.clone(),
            |mut snapshot| {
                for index in 0..1_000 {
                    snapshot.update(|snapshot| snapshot.0[index] = index);
                }
            },
            BatchSize::SmallInput,
        );
    });

    let cached = TrackedBox::<Snapshot, CachedAccount, _>::new_with_account(Snapshot(vec![0; 1_000]), budget());
    group.bench_function("snapshots/cached_updates/1000", |bencher| {
        bencher.iter_batched(
            || cached.clone(),
            |mut snapshot| {
                for index in 0..1_000 {
                    snapshot.update(|snapshot| snapshot.0[index] = index);
                }
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

criterion_group!(benches, lifecycle);
criterion_main!(benches);
