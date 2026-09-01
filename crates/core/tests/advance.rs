use std::time::Duration;

use contime_core::{checkpoints, ConTime, ConTimeConfig, Input, RejectionReason};
use contime_memory::ConservativeTrackedSize;
use crossbeam_channel::{unbounded, TryRecvError};

struct TestEvent {
    id: u128,
    time: u64,
    value: u64,
    snapshot_id: u128,
}

impl ConservativeTrackedSize for TestEvent {
    fn conservative_tracked_size(&self) -> usize {
        128
    }
}

impl Input for TestEvent {
    type Time = u64;

    fn event_id(&self) -> u128 {
        self.id
    }

    fn time(&self) -> u64 {
        self.time
    }

    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        emit(self.snapshot_id);
    }
}

#[derive(Clone, Default)]
struct TestSnapshot {
    snapshot_id: u128,
    time: u64,
    value: u64,
}

impl ConservativeTrackedSize for TestSnapshot {
    fn conservative_tracked_size(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

impl checkpoints::Snapshot for TestSnapshot {
    type Time = u64;

    fn set_time(&mut self, time: u64) {
        self.time = time;
    }
}

impl checkpoints::ApplyEvents<TestEvent> for TestSnapshot {
    fn create(snapshot_id: u128, _first_event: &TestEvent) -> Self {
        Self { snapshot_id, ..Self::default() }
    }

    fn apply_events(&mut self, batch: checkpoints::ApplyBatch<'_, u64, TestEvent>) {
        self.value += batch.events.iter().map(|event| event.value).sum::<u64>();
    }
}

fn config(router_count: usize, worker_count: usize, retention: u64) -> ConTimeConfig<u64> {
    config_with_replays(router_count, worker_count, retention, 1)
}

fn config_with_replays(router_count: usize, worker_count: usize, retention: u64, replays_per_receive: usize) -> ConTimeConfig<u64> {
    ConTimeConfig {
        router_count,
        worker_count,
        router_seed: 9,
        memory_limit: 10_000_000,
        memory_buffer: 1_000,
        history_retention: retention,
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
    TestEvent { id, time, value, snapshot_id }
}

#[test]
fn advance_preserves_state_releases_memory_and_rejects_late_old_events() {
    let contime = ConTime::<TestEvent, TestSnapshot, ()>::start(config(1, 1, 10), ()).unwrap();
    assert!(contime.apply([event(1, 1, 1), event(2, 5, 1), event(3, 10, 1), event(4, 15, 1)]).unwrap().is_empty());
    let before = contime.used_memory();

    contime.advance_to(20).unwrap();

    let snapshot = contime.query_at(20, [7]).unwrap().pop().unwrap();
    assert_eq!(snapshot.snapshot_id, 7);
    assert_eq!(snapshot.value, 4);
    assert!(contime.used_memory() < before);

    let rejected = contime.apply([event(99, 9, 1)]).unwrap();
    assert_eq!(rejected.len(), 1);
    assert_eq!(rejected[0].event_id, 99);
    assert_eq!(rejected[0].reason, RejectionReason::BeforeHistoryHorizon);
    assert!(contime.apply([event(100, 10, 1)]).unwrap().is_empty());
    contime.shutdown();
}

#[test]
fn repeated_and_backward_advances_are_no_ops_and_pruned_ids_can_be_reused() {
    let contime = ConTime::<TestEvent, TestSnapshot, ()>::start(config(1, 1, 10), ()).unwrap();
    contime.apply([event(1, 1, 1), event(2, 10, 1)]).unwrap();
    contime.advance_to(20).unwrap();
    let after_first = contime.used_memory();

    contime.advance_to(20).unwrap();
    contime.advance_to(15).unwrap();

    assert_eq!(contime.used_memory(), after_first);
    assert!(contime.apply([event(1, 10, 3)]).unwrap().is_empty());
    assert_eq!(contime.query_at(20, [7]).unwrap().pop().unwrap().value, 5);
    contime.shutdown();
}

#[test]
fn asynchronous_advance_closes_after_every_worker_and_old_queries_use_the_anchor() {
    let contime = ConTime::<TestEvent, TestSnapshot, ()>::start(config(2, 4, 10), ()).unwrap();
    contime.apply([event(1, 1, 1), event(2, 5, 1), event(3, 10, 1)]).unwrap();
    let (completion, done) = unbounded();

    contime.send_advance_to(20, completion).unwrap();

    assert_eq!(done.into_iter().collect::<Vec<_>>(), Vec::<()>::new());
    let old = contime.query_at(0, [7]).unwrap().pop().unwrap();
    assert_eq!(old.time, 5);
    assert_eq!(old.value, 2);
    contime.shutdown();
}

#[test]
fn advance_forces_dirty_pre_horizon_replay_before_pruning() {
    let contime = ConTime::<TestEvent, TestSnapshot, ()>::start(config_with_replays(1, 1, 10, 0), ()).unwrap();
    let (completion, applied) = unbounded();
    contime.send([event(1, 5, 7)], completion).unwrap();

    contime.advance_to(20).unwrap();

    assert_eq!(applied.into_iter().collect::<Vec<_>>(), Vec::new());
    assert_eq!(contime.query_at(20, [7]).unwrap().pop().unwrap().value, 7);
    contime.shutdown();
}

#[test]
fn dirty_event_at_the_horizon_remains_scheduled() {
    let contime = ConTime::<TestEvent, TestSnapshot, ()>::start(config_with_replays(1, 1, 10, 0), ()).unwrap();
    let (completion, applied) = unbounded();
    contime.send([event(1, 10, 7)], completion).unwrap();

    contime.advance_to(20).unwrap();

    assert_eq!(applied.try_recv(), Err(TryRecvError::Empty));
    assert_eq!(contime.query_at(20, [7]).unwrap().pop().unwrap().value, 7);
    contime.shutdown();
    assert_eq!(applied.try_recv(), Err(TryRecvError::Disconnected));
}

#[test]
fn a_history_first_seen_after_advancement_starts_at_the_active_horizon() {
    let contime = ConTime::<TestEvent, TestSnapshot, ()>::start(config(1, 1, 10), ()).unwrap();
    contime.advance_to(20).unwrap();

    let rejected = contime.apply([event_for(9, 1, 9, 1)]).unwrap();
    assert_eq!(rejected[0].reason, RejectionReason::BeforeHistoryHorizon);
    assert!(contime.apply([event_for(9, 2, 10, 3)]).unwrap().is_empty());
    assert_eq!(contime.query_at(20, [9]).unwrap().pop().unwrap().value, 3);
    contime.shutdown();
}
