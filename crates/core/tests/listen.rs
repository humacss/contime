use std::collections::BTreeSet;
use std::time::Duration;

use contime_core::checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Snapshot};
use contime_core::{ConTime, ConTimeConfig, Input, RejectionMessage, RejectionReason, SnapshotListenerMessage};

use crossbeam_channel::unbounded;

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

struct TestEvent {
    id: u128,
    snapshot_id: u128,
    time: Time,
}

impl contime_checkpoints::Event for TestEvent {
    type Time = Time;

    fn time(&self) -> Self::Time {
        self.time
    }
}
impl Input for TestEvent {
    fn event_id(&self) -> u128 {
        self.id
    }
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        emit(self.snapshot_id);
    }
}

#[derive(Clone, Default)]
struct TestSnapshot {
    time: Time,
    count: usize,
}

impl Snapshot for TestSnapshot {
    type Time = Time;

    fn time(&self) -> &Self::Time {
        &self.time
    }
    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }
}

impl ApplyEvents<TestEvent> for TestSnapshot {
    fn create(_snapshot_id: u128, _first_event: &TestEvent) -> Self {
        Self::default()
    }

    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Self::Time, TestEvent>) {
        self.count += batch.events.count();
    }
}

fn config() -> ConTimeConfig<Time> {
    ConTimeConfig {
        router_count: 2,
        worker_count: 4,
        placement: contime_router::Placement::default(),
        pruning_interval: std::time::Duration::from_millis(100),

        history_retention: Time(100),
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

    contime.send_listen_snapshots(Time(10), snapshot_ids.iter().copied(), notifications).unwrap();

    let mut registered = BTreeSet::new();
    while registered.len() < snapshot_ids.len() {
        let SnapshotListenerMessage::Registered { time, snapshot_ids } = observed.recv_timeout(Duration::from_secs(1)).unwrap() else {
            panic!("replay arrived before the measured event")
        };
        assert_eq!(time, Time(10));
        registered.extend(snapshot_ids);
    }
    assert_eq!(registered, snapshot_ids.iter().copied().collect());

    let (rejections, completed) = unbounded::<RejectionMessage<RejectionReason>>();
    contime
        .send(snapshot_ids.iter().map(|&snapshot_id| TestEvent { id: snapshot_id + 1, snapshot_id, time: Time(10) }), rejections)
        .unwrap();
    contime.advance_to(Time(10)).unwrap();

    let mut replayed = BTreeSet::new();
    while replayed.len() < snapshot_ids.len() {
        let SnapshotListenerMessage::Replayed { time, snapshot_ids } = observed.recv_timeout(Duration::from_secs(1)).unwrap() else {
            panic!("unexpected registration acknowledgement")
        };
        assert_eq!(time, Time(10));
        replayed.extend(snapshot_ids);
    }
    assert_eq!(replayed, snapshot_ids.iter().copied().collect());
    assert!(completed.into_iter().collect::<Vec<_>>().is_empty());

    let (rejections, completed) = unbounded::<RejectionMessage<RejectionReason>>();
    contime.send([TestEvent { id: 1, snapshot_id: 0, time: Time(0) }], rejections).unwrap();
    assert!(completed.into_iter().collect::<Vec<_>>().is_empty());
    contime.wait_until_idle(Duration::from_secs(2)).unwrap();
    assert!(observed.is_empty());

    let (rejections, completed) = unbounded::<RejectionMessage<RejectionReason>>();
    contime
        .send(snapshot_ids.iter().map(|&snapshot_id| TestEvent { id: 1_000 + snapshot_id, snapshot_id, time: Time(11) }), rejections)
        .unwrap();
    contime.advance_to(Time(11)).unwrap();
    assert!(completed.into_iter().collect::<Vec<_>>().is_empty());
    contime.wait_until_idle(Duration::from_secs(2)).unwrap();
    assert!(observed.is_empty());
    assert!(contime.query_at(Time(11), snapshot_ids).unwrap().iter().all(|snapshot| snapshot.count == 2));
    contime.shutdown();
}
