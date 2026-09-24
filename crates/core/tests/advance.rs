use std::time::Duration;

use contime_core::{checkpoints, ConTime, ConTimeConfig, Input, RejectionReason};

use crossbeam_channel::{unbounded, TryRecvError};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
struct Time(u64);

impl checkpoints::Timestamp for Time {
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
    time: Time,
    value: u64,
    snapshot_id: u128,
}

impl contime_checkpoints::Event for TestEvent {
    type Time = Time;

    fn time(&self) -> Time {
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
    snapshot_id: u128,
    time: Time,
    value: u64,
}

impl checkpoints::Snapshot for TestSnapshot {
    type Time = Time;

    fn time(&self) -> &Self::Time {
        &self.time
    }
    fn set_time(&mut self, time: Time) {
        self.time = time;
    }
}

impl checkpoints::ApplyEvents<TestEvent> for TestSnapshot {
    fn create(snapshot_id: u128, _first_event: &TestEvent) -> Self {
        Self { snapshot_id, ..Self::default() }
    }

    fn apply_events(&mut self, batch: checkpoints::ApplyBatch<'_, '_, Time, TestEvent>) {
        self.value += batch.events.map(|event| event.value).sum::<u64>();
    }
}

fn config(router_count: usize, worker_count: usize, retention: u64) -> ConTimeConfig<Time> {
    config_with_replays(router_count, worker_count, retention, 1)
}

fn config_with_replays(router_count: usize, worker_count: usize, retention: u64, replays_per_receive: usize) -> ConTimeConfig<Time> {
    ConTimeConfig {
        router_count,
        worker_count,
        placement: contime_router::Placement::default(),
        pruning_interval: std::time::Duration::from_millis(100),

        history_retention: Time(retention),
        worker: contime_worker::WorkerConfig {
            maximum_dirty_age: Duration::from_micros(100),
            replays_per_receive,
            deadline_compaction_minimum: 1_024,
            deadline_compaction_multiplier: 2,
        },
        checkpoints: checkpoints::CheckpointConfig { interval: 2 },
    }
}

fn event(id: u128, time: u64, value: u64) -> TestEvent {
    event_for(7, id, time, value)
}

fn event_for(snapshot_id: u128, id: u128, time: u64, value: u64) -> TestEvent {
    TestEvent { id, time: Time(time), value, snapshot_id }
}

#[test]
fn pruned_horizon_subscribers_receive_completed_progress_and_late_initial_value() {
    let contime = ConTime::<TestEvent, TestSnapshot, ()>::start(config(2, 3, 10), ()).unwrap();
    let first = contime.subscribe_pruned_horizon().unwrap();
    let second = contime.subscribe_pruned_horizon().unwrap();
    assert_eq!(first.recv_timeout(Duration::from_secs(2)).unwrap(), Time(0));
    assert_eq!(second.recv_timeout(Duration::from_secs(2)).unwrap(), Time(0));
    contime.apply([event_for(0, 1, 1, 1), event_for(1, 2, 1, 1)]).unwrap();
    contime.advance_to(Time(20)).unwrap();
    contime.wait_until_idle(Duration::from_secs(2)).unwrap();
    for receiver in [&first, &second] {
        let mut last = Time(0);
        while last < Time(10) {
            let next = receiver.recv_timeout(Duration::from_secs(2)).unwrap();
            assert!(next > last);
            last = next;
        }
        assert_eq!(last, Time(10));
    }
    let late = contime.subscribe_pruned_horizon().unwrap();
    assert_eq!(late.recv_timeout(Duration::from_secs(2)).unwrap(), Time(10));
    contime.advance_to(Time(20)).unwrap();
    contime.wait_until_idle(Duration::from_secs(2)).unwrap();
    assert!(late.is_empty());
    contime.shutdown();
    assert_eq!(late.recv(), Err(crossbeam_channel::RecvError));
}

#[test]
fn advance_preserves_state_and_rejects_late_old_events() {
    let contime = ConTime::<TestEvent, TestSnapshot, ()>::start(config(1, 1, 10), ()).unwrap();
    contime.apply([event(1, 1, 1), event(2, 5, 1), event(3, 10, 1), event(4, 15, 1)]).unwrap();
    contime.wait_until_idle(Duration::from_secs(2)).unwrap();
    assert!(contime.errors().is_empty());

    contime.advance_to(Time(20)).unwrap();
    contime.wait_until_idle(Duration::from_secs(2)).unwrap();

    let snapshot = contime.query_at(Time(20), [7]).unwrap().pop().unwrap();
    assert_eq!(snapshot.snapshot_id, 7);
    assert_eq!(snapshot.value, 4);

    contime.apply([event(99, 9, 1)]).unwrap();
    contime.wait_until_idle(Duration::from_secs(2)).unwrap();
    let rejected = contime.errors().try_iter().collect::<Vec<_>>();
    assert_eq!(rejected.len(), 1);
    assert_eq!(rejected[0].event_id, 99);
    assert_eq!(rejected[0].reason, RejectionReason::BeforeHistoryHorizon);
    contime.apply([event(100, 10, 1)]).unwrap();
    contime.wait_until_idle(Duration::from_secs(2)).unwrap();
    assert!(contime.errors().is_empty());
    contime.shutdown();
}

#[test]
fn repeated_and_backward_advances_are_no_ops_and_pruned_ids_can_be_reused() {
    let mut settings = config(1, 1, 10);
    settings.pruning_interval = Duration::ZERO;
    let contime = ConTime::<TestEvent, TestSnapshot, ()>::start(settings, ()).unwrap();
    contime.apply([event(1, 1, 1), event(2, 10, 1)]).unwrap();
    contime.advance_to(Time(20)).unwrap();
    contime.wait_until_idle(Duration::from_secs(2)).unwrap();

    contime.advance_to(Time(20)).unwrap();
    contime.advance_to(Time(15)).unwrap();
    contime.wait_until_idle(Duration::from_secs(2)).unwrap();

    contime.apply([event(1, 10, 3)]).unwrap();
    contime.wait_until_idle(Duration::from_secs(2)).unwrap();
    assert!(contime.errors().is_empty());
    assert_eq!(contime.query_at(Time(20), [7]).unwrap().pop().unwrap().value, 5);
    contime.shutdown();
}

#[test]
fn advance_admission_closes_and_old_queries_return_no_snapshot() {
    let contime = ConTime::<TestEvent, TestSnapshot, ()>::start(config(2, 4, 10), ()).unwrap();
    contime.apply([event(1, 1, 1), event(2, 5, 1), event(3, 10, 1)]).unwrap();
    let (completion, done) = unbounded();

    contime.send_advance_to(Time(20), completion).unwrap();

    assert_eq!(done.into_iter().collect::<Vec<_>>(), Vec::<()>::new());
    contime.wait_until_idle(Duration::from_secs(2)).unwrap();
    assert!(contime.query_at(Time(0), [7]).unwrap().is_empty());
    contime.shutdown();
}

#[test]
fn advance_forces_dirty_pre_horizon_replay_before_pruning() {
    let contime = ConTime::<TestEvent, TestSnapshot, ()>::start(config_with_replays(1, 1, 10, 0), ()).unwrap();
    let (completion, applied) = unbounded();
    contime.send([event(1, 5, 7)], completion).unwrap();

    contime.advance_to(Time(20)).unwrap();

    assert_eq!(applied.into_iter().collect::<Vec<_>>(), Vec::new());
    contime.wait_until_idle(Duration::from_secs(2)).unwrap();
    assert_eq!(contime.query_at(Time(20), [7]).unwrap().pop().unwrap().value, 7);
    contime.shutdown();
}

#[test]
fn event_at_the_horizon_remains_available_after_immediate_replay() {
    let contime = ConTime::<TestEvent, TestSnapshot, ()>::start(config_with_replays(1, 1, 10, 0), ()).unwrap();
    let (completion, applied) = unbounded();
    contime.send([event(1, 10, 7)], completion).unwrap();

    contime.advance_to(Time(20)).unwrap();

    contime.wait_until_idle(Duration::from_secs(2)).unwrap();
    assert_eq!(applied.try_recv(), Err(TryRecvError::Disconnected));
    assert_eq!(contime.query_at(Time(20), [7]).unwrap().pop().unwrap().value, 7);
    contime.shutdown();
    assert_eq!(applied.try_recv(), Err(TryRecvError::Disconnected));
}

#[test]
fn a_history_first_seen_after_advancement_starts_at_the_active_horizon() {
    let contime = ConTime::<TestEvent, TestSnapshot, ()>::start(config(1, 1, 10), ()).unwrap();
    contime.advance_to(Time(20)).unwrap();
    contime.wait_until_idle(Duration::from_secs(2)).unwrap();

    contime.apply([event_for(9, 1, 9, 1)]).unwrap();
    contime.wait_until_idle(Duration::from_secs(2)).unwrap();
    let rejected = contime.errors().try_iter().collect::<Vec<_>>();
    assert_eq!(rejected.len(), 1);
    assert_eq!(rejected[0].event_id, 1);
    assert_eq!(rejected[0].reason, RejectionReason::BeforeHistoryHorizon);
    contime.apply([event_for(9, 2, 10, 3)]).unwrap();
    contime.wait_until_idle(Duration::from_secs(2)).unwrap();
    assert!(contime.errors().is_empty());
    assert_eq!(contime.query_at(Time(20), [9]).unwrap().pop().unwrap().value, 3);
    contime.shutdown();
}
