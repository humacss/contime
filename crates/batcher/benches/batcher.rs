use contime_batcher::{drain, Batch, Batcher, Request, MAX_EVENTS};
use criterion::{black_box, criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use std::{collections::VecDeque, sync::Arc, time::Duration};

const ITEMS: usize = 16_384;
struct Segment {
    items: Vec<usize>,
    completion: Arc<()>,
}
impl Batch for Segment {
    type Item = usize;
    type Context = Arc<()>;
    fn into_parts(self) -> (Vec<usize>, Arc<()>) {
        (self.items, self.completion)
    }
    fn from_parts(items: Vec<usize>, completion: Arc<()>) -> Self {
        Self { items, completion }
    }
    fn items(&mut self) -> &mut Vec<usize> {
        &mut self.items
    }
    fn can_merge(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.completion, &other.completion)
    }
}
enum Message {
    Apply(Segment),
    Boundary,
}
impl Request for Message {
    type Batch = Segment;
    fn into_batch(self) -> Result<Segment, Self> {
        match self {
            Self::Apply(b) => Ok(b),
            other => Err(other),
        }
    }
}
fn submissions(size: usize, distinct: bool) -> Vec<Segment> {
    let shared = Arc::new(());
    (0..ITEMS)
        .step_by(size)
        .map(|start| Segment {
            items: (start..(start + size).min(ITEMS)).collect(),
            completion: if distinct { Arc::new(()) } else { shared.clone() },
        })
        .collect()
}
fn benchmarks(c: &mut Criterion) {
    for mode in ["packetize", "drain"] {
        let mut group = c.benchmark_group(format!("batcher/{mode}"));
        group.throughput(Throughput::Elements(ITEMS as u64));
        group.sample_size(10).warm_up_time(Duration::from_secs(1)).measurement_time(Duration::from_secs(1));
        for distinct in [false, true] {
            for size in [1, 1_000] {
                let name = if distinct { "distinct-responses" } else { "shared-response" };
                group.bench_with_input(BenchmarkId::new(name, size), &size, |bencher, &size| {
                    if mode == "packetize" {
                        bencher.iter_batched(
                            || submissions(size, distinct),
                            |submissions| {
                                let mut packets = Vec::new();
                                let mut batcher = Batcher::new(
                                    |p| {
                                        packets.push(p);
                                        Ok::<_, ()>(())
                                    },
                                    MAX_EVENTS,
                                );
                                for batch in submissions {
                                    batcher.push(black_box(batch)).unwrap();
                                }
                                batcher.flush().unwrap();
                                drop(batcher);
                                black_box(packets)
                            },
                            BatchSize::SmallInput,
                        );
                    } else {
                        bencher.iter_batched(
                            || {
                                let mut queue: VecDeque<_> = submissions(size, distinct).into_iter().map(Message::Apply).collect();
                                queue.push_back(Message::Boundary);
                                queue
                            },
                            |mut queue| {
                                let mut packets = Vec::new();
                                while let Some(Message::Apply(first)) = queue.pop_front() {
                                    drain(
                                        first,
                                        || queue.pop_front(),
                                        |p| {
                                            packets.push(p);
                                            Ok::<_, ()>(())
                                        },
                                        Some,
                                        || true,
                                    )
                                    .unwrap();
                                }
                                black_box(packets)
                            },
                            BatchSize::SmallInput,
                        );
                    }
                });
            }
        }
        group.finish();
    }
}
criterion_group!(benches, benchmarks);
criterion_main!(benches);
