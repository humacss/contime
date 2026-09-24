use std::collections::BTreeSet;
use std::time::Duration;

use contime_core::checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Snapshot};
use contime_core::{ConTime, ConTimeConfig, Input, RejectionMessage, RejectionReason, SnapshotListenerMessage};

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use crossbeam_channel::{unbounded, Receiver};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
struct Time(u64);

impl contime_core::checkpoints::Timestamp for Time {
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}

impl contime_worker::AdvanceTime for Time {
    fn saturating_sub(&self, retention: &Self) -> Self {
        Self(self.0.saturating_sub(retention.0))
    }
}

const EVENTS_PER_BATCH: usize = 1_000;
const BATCHES_PER_SAMPLE: usize = 100;

struct BenchEvent {
    id: u128,
    snapshot_id: u128,
}

impl contime_checkpoints::Event for BenchEvent {
    type Time = Time;

    fn time(&self) -> Self::Time {
        Time(1)
    }
}
impl Input for BenchEvent {
    fn event_id(&self) -> u128 {
        self.id
    }
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        emit(self.snapshot_id);
    }
}

#[derive(Clone, Default)]
struct BenchSnapshot {
    time: Time,
    count: usize,
}

impl Snapshot for BenchSnapshot {
    type Time = Time;

    fn time(&self) -> &Self::Time {
        &self.time
    }
    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }
}

impl ApplyEvents<BenchEvent> for BenchSnapshot {
    fn create(_snapshot_id: u128, _first_event: &BenchEvent) -> Self {
        Self::default()
    }

    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Self::Time, BenchEvent>) {
        self.count += batch.events.count();
    }
}

fn config(router_count: usize, worker_count: usize) -> ConTimeConfig<Time> {
    ConTimeConfig {
        router_count,
        worker_count,
        placement: contime_router::Placement::default(),
        pruning_interval: std::time::Duration::from_millis(100),

        history_retention: Time(0),
        worker: contime_worker::WorkerConfig {
            maximum_dirty_age: Duration::from_secs(60),
            replays_per_receive: usize::MAX,
            deadline_compaction_minimum: 1_024,
            deadline_compaction_multiplier: 2,
        },
        checkpoints: CheckpointConfig { interval: 100 },
    }
}

fn receive_registration_batches(receiver: &Receiver<SnapshotListenerMessage<Time>>, expected_ids: usize) -> usize {
    let mut batches = 0;
    let mut registered = BTreeSet::new();
    while registered.len() < expected_ids {
        let SnapshotListenerMessage::Registered { time, snapshot_ids } = receiver.recv().unwrap() else {
            panic!("replay arrived while listener registration was incomplete")
        };
        assert_eq!(time, Time(u64::MAX));
        registered.extend(snapshot_ids);
        batches += 1;
    }
    assert_eq!(registered.len(), expected_ids);
    batches
}

fn receive_replay_batches(receiver: &Receiver<SnapshotListenerMessage<Time>>, expected_ids: usize) {
    let mut replayed = BTreeSet::new();
    // Called after idle: coalescing and timestamp steps determine notification
    // count, so verify every affected snapshot instead of assuming batch count.
    for message in receiver.try_iter() {
        let SnapshotListenerMessage::Replayed { time, snapshot_ids } = message else {
            panic!("unexpected registration acknowledgement in measured workload")
        };
        assert_eq!(time, Time(u64::MAX));
        assert!(!snapshot_ids.is_empty());
        replayed.extend(snapshot_ids);
    }
    assert_eq!(replayed, (0..expected_ids as u128).collect());
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
    contime.advance_to(Time(1)).unwrap();
    let (rejections, completed) = unbounded::<RejectionMessage<RejectionReason>>();
    contime.send(prepare_batches(snapshot_count, 1, next_id).pop().unwrap(), rejections).unwrap();
    assert!(completed.into_iter().next().is_none());
    contime.wait_until_idle(Duration::from_secs(5)).unwrap();
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
                        contime.send_listen_snapshots(Time(u64::MAX), 0..snapshot_count as u128, notifications).unwrap();
                        receive_registration_batches(&observed, snapshot_count);
                        observed
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
                            contime.wait_until_idle(Duration::from_secs(5)).unwrap();
                            if let Some(observed) = &listener {
                                receive_replay_batches(observed, snapshot_count);
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
