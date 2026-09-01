use std::collections::BTreeSet;
use std::time::Duration;

use contime_checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Snapshot};
use contime_core::{ConTime, ConTimeConfig, Input, RejectionMessage, RejectionReason, SnapshotListenerMessage};
use contime_memory::ConservativeTrackedSize;
use crossbeam_channel::unbounded;

struct TestEvent {
    id: u128,
    snapshot_id: u128,
    time: u64,
}

impl ConservativeTrackedSize for TestEvent {
    fn conservative_tracked_size(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

impl Input for TestEvent {
    type Time = u64;

    fn event_id(&self) -> u128 {
        self.id
    }

    fn time(&self) -> Self::Time {
        self.time
    }

    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        emit(self.snapshot_id);
    }
}

#[derive(Clone, Default)]
struct TestSnapshot {
    time: u64,
    count: usize,
}

impl ConservativeTrackedSize for TestSnapshot {
    fn conservative_tracked_size(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

impl Snapshot for TestSnapshot {
    type Time = u64;

    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }
}

impl ApplyEvents<TestEvent> for TestSnapshot {
    fn create(_snapshot_id: u128, _first_event: &TestEvent) -> Self {
        Self::default()
    }

    fn apply_events(&mut self, batch: ApplyBatch<'_, Self::Time, TestEvent>) {
        self.count += batch.events.len();
    }
}

fn config() -> ConTimeConfig<u64> {
    ConTimeConfig {
        router_count: 2,
        worker_count: 4,
        router_seed: 9,
        memory_limit: 1_000_000,
        memory_buffer: 1_000,
        history_retention: 0,
        worker: contime_worker::WorkerConfig {
            maximum_dirty_age: Duration::from_micros(100),
            replays_per_receive: 1,
            deadline_compaction_minimum: 1_024,
            deadline_compaction_multiplier: 2,
        },
        checkpoints: CheckpointConfig { interval: 100 },
    }
}

#[test]
fn public_listener_collection_batches_matching_replays_and_ignores_later_events() {
    let contime = ConTime::<TestEvent, TestSnapshot, ()>::start(config(), ()).unwrap();
    let (notifications, observed) = unbounded();
    let snapshot_ids = (0..100_u128).collect::<Vec<_>>();

    contime.send_listen_snapshots(10, snapshot_ids.iter().copied(), notifications).unwrap();

    let mut registered = BTreeSet::new();
    while registered.len() < snapshot_ids.len() {
        let SnapshotListenerMessage::Registered { time, snapshot_ids } = observed.recv_timeout(Duration::from_secs(1)).unwrap() else {
            panic!("replay arrived before the measured event")
        };
        assert_eq!(time, 10);
        registered.extend(snapshot_ids);
    }
    assert_eq!(registered, snapshot_ids.iter().copied().collect());

    let (rejections, completed) = unbounded::<RejectionMessage<RejectionReason>>();
    contime.send(snapshot_ids.iter().map(|&snapshot_id| TestEvent { id: snapshot_id + 1, snapshot_id, time: 10 }), rejections).unwrap();

    let mut replayed = BTreeSet::new();
    while replayed.len() < snapshot_ids.len() {
        let SnapshotListenerMessage::Replayed { time, snapshot_ids } = observed.recv_timeout(Duration::from_secs(1)).unwrap() else {
            panic!("unexpected registration acknowledgement")
        };
        assert_eq!(time, 10);
        replayed.extend(snapshot_ids);
    }
    assert_eq!(replayed, snapshot_ids.iter().copied().collect());
    assert!(completed.into_iter().collect::<Vec<_>>().is_empty());

    let (rejections, completed) = unbounded::<RejectionMessage<RejectionReason>>();
    contime.send([TestEvent { id: 1, snapshot_id: 0, time: 0 }], rejections).unwrap();
    assert!(completed.into_iter().collect::<Vec<_>>().is_empty());
    assert!(observed.recv_timeout(Duration::from_millis(20)).is_err());

    let (rejections, completed) = unbounded::<RejectionMessage<RejectionReason>>();
    contime.send(snapshot_ids.iter().map(|&snapshot_id| TestEvent { id: 1_000 + snapshot_id, snapshot_id, time: 11 }), rejections).unwrap();
    assert!(completed.into_iter().collect::<Vec<_>>().is_empty());
    assert!(observed.recv_timeout(Duration::from_millis(20)).is_err());
    contime.shutdown();
}
