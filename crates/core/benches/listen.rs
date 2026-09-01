use std::collections::BTreeSet;
use std::time::Duration;

use contime_checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Snapshot};
use contime_core::{ConTime, ConTimeConfig, Input, RejectionMessage, RejectionReason, SnapshotListenerMessage};
use contime_memory::ConservativeTrackedSize;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use crossbeam_channel::{unbounded, Receiver};

const EVENTS_PER_BATCH: usize = 1_000;
const BATCHES_PER_SAMPLE: usize = 100;

struct BenchEvent {
    id: u128,
    snapshot_id: u128,
}

impl ConservativeTrackedSize for BenchEvent {
    fn conservative_tracked_size(&self) -> usize {
        64
    }
}

impl Input for BenchEvent {
    type Time = u64;

    fn event_id(&self) -> u128 {
        self.id
    }

    fn time(&self) -> Self::Time {
        1
    }

    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        emit(self.snapshot_id);
    }
}

#[derive(Clone, Default)]
struct BenchSnapshot {
    time: u64,
    count: usize,
}

impl ConservativeTrackedSize for BenchSnapshot {
    fn conservative_tracked_size(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

impl Snapshot for BenchSnapshot {
    type Time = u64;

    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }
}

impl ApplyEvents<BenchEvent> for BenchSnapshot {
    fn create(_snapshot_id: u128, _first_event: &BenchEvent) -> Self {
        Self::default()
    }

    fn apply_events(&mut self, batch: ApplyBatch<'_, Self::Time, BenchEvent>) {
        self.count += batch.events.len();
    }
}

fn config(router_count: usize, worker_count: usize) -> ConTimeConfig<u64> {
    ConTimeConfig {
        router_count,
        worker_count,
        router_seed: 9,
        memory_limit: usize::MAX,
        memory_buffer: 0,
        history_retention: 0,
        worker: contime_worker::WorkerConfig {
            maximum_dirty_age: Duration::from_secs(60),
            replays_per_receive: usize::MAX,
            deadline_compaction_minimum: 1_024,
            deadline_compaction_multiplier: 2,
        },
        checkpoints: CheckpointConfig { interval: 100 },
    }
}

fn receive_registration_batches(receiver: &Receiver<SnapshotListenerMessage<u64>>, expected_ids: usize) -> usize {
    let mut batches = 0;
    let mut registered = BTreeSet::new();
    while registered.len() < expected_ids {
        let SnapshotListenerMessage::Registered { time, snapshot_ids } = receiver.recv().unwrap() else {
            panic!("replay arrived while listener registration was incomplete")
        };
        assert_eq!(time, u64::MAX);
        registered.extend(snapshot_ids);
        batches += 1;
    }
    assert_eq!(registered.len(), expected_ids);
    batches
}

fn receive_replay_batches(receiver: &Receiver<SnapshotListenerMessage<u64>>, expected_batches: usize) {
    for _ in 0..expected_batches {
        let SnapshotListenerMessage::Replayed { time, snapshot_ids } = receiver.recv().unwrap() else {
            panic!("unexpected registration acknowledgement in measured workload")
        };
        assert_eq!(time, u64::MAX);
        assert!(!snapshot_ids.is_empty());
    }
}

fn prepare_batches(snapshot_count: usize, batch_count: usize, next_id: &mut u128) -> Vec<Vec<BenchEvent>> {
    (0..batch_count)
        .map(|_| {
            (0..EVENTS_PER_BATCH)
                .map(|event_index| {
                    let event = BenchEvent { id: *next_id, snapshot_id: (event_index % snapshot_count) as u128 };
                    *next_id += 1;
                    event
                })
                .collect()
        })
        .collect()
}

fn warm_runtime(contime: &ConTime<BenchEvent, BenchSnapshot, ()>, snapshot_count: usize, next_id: &mut u128) {
    let (rejections, completed) = unbounded::<RejectionMessage<RejectionReason>>();
    contime.send(prepare_batches(snapshot_count, 1, next_id).pop().unwrap(), rejections).unwrap();
    assert!(completed.into_iter().next().is_none());
}

fn benchmark_replay_overhead(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("listen/sustained_replay");
    for (routers, workers) in [(1, 1), (2, 4)] {
        for snapshot_count in [1usize, 100, 1_000] {
            let batch_count = BATCHES_PER_SAMPLE;
            let event_count = batch_count * EVENTS_PER_BATCH;
            group.throughput(Throughput::Elements(event_count as u64));

            for listeners_enabled in [false, true] {
                let mode = if listeners_enabled { "enabled" } else { "baseline" };
                let benchmark_id = BenchmarkId::new(format!("{routers}_routers_{workers}_workers/{mode}"), snapshot_count);
                group.bench_function(benchmark_id, |bencher| {
                    let contime = ConTime::<BenchEvent, BenchSnapshot, ()>::start(config(routers, workers), ()).unwrap();
                    let mut next_id = 1;
                    warm_runtime(&contime, snapshot_count, &mut next_id);

                    let listener = listeners_enabled.then(|| {
                        let (notifications, observed) = unbounded();
                        contime.send_listen_snapshots(u64::MAX, 0..snapshot_count as u128, notifications).unwrap();
                        let worker_batch_count = receive_registration_batches(&observed, snapshot_count);
                        (observed, worker_batch_count)
                    });

                    bencher.iter_batched(
                        || prepare_batches(snapshot_count, batch_count, &mut next_id),
                        |batches| {
                            let (completion, completed) = unbounded::<RejectionMessage<RejectionReason>>();
                            for batch in batches {
                                contime.send(batch, completion.clone()).unwrap();
                            }
                            drop(completion);
                            assert!(completed.into_iter().next().is_none());
                            if let Some((observed, worker_batch_count)) = &listener {
                                receive_replay_batches(observed, batch_count * worker_batch_count);
                            }
                        },
                        criterion::BatchSize::LargeInput,
                    );
                    contime.shutdown();
                });
            }
        }
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10).warm_up_time(Duration::from_secs(1)).measurement_time(Duration::from_secs(3));
    targets = benchmark_replay_overhead
}
criterion_main!(benches);
