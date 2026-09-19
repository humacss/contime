use std::time::Duration;

use contime_core::{checkpoints, ConTime, ConTimeConfig, Input};
use contime_memory::ConservativeTrackedSize;

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
        placement: contime_router::Placement::default(),
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
fn idle_wait_finishes_all_queued_batches_without_advancing_time() {
    let core = ConTime::<TestEvent, TestSnapshot, ()>::start(config(3, 4, 1_000), ()).unwrap();
    core.wait_until_idle(Duration::from_secs(2)).unwrap();
    for id in 0..128 {
        let (tx, _rx) = crossbeam_channel::unbounded();
        core.send([event_for(id, id, 10, 1)], tx).unwrap();
    }
    core.wait_until_idle(Duration::from_secs(2)).unwrap();
    let snapshots = core.query_at(10, 0..128).unwrap();
    assert_eq!(snapshots.len(), 128);
    assert!(snapshots.iter().all(|snapshot| snapshot.value == 1));
    // Waiting must not prune or advance history.
    core.apply([event_for(0, 999, 0, 7)]).unwrap();
    core.wait_until_idle(Duration::from_secs(2)).unwrap();
    assert!(core.errors().is_empty());
    assert_eq!(core.query_at(10, [0]).unwrap()[0].value, 8);
    core.shutdown();
}

#[test]
fn repeated_idle_subscriptions_do_not_stop_later_work() {
    let core = ConTime::<TestEvent, TestSnapshot, ()>::start(config(2, 3, 1_000), ()).unwrap();
    for round in 0..20 {
        core.wait_until_idle(Duration::from_secs(2)).unwrap();
        let (tx, _rx) = crossbeam_channel::unbounded();
        core.send([event_for(7, round, 10, 1)], tx).unwrap();
        core.wait_until_idle(Duration::from_secs(2)).unwrap();
        assert_eq!(core.query_at(10, [7]).unwrap()[0].value, round as u64 + 1);
    }
    core.shutdown();
}

#[test]
fn independent_callers_can_wait_for_the_same_idle_topology() {
    let core = ConTime::<TestEvent, TestSnapshot, ()>::start(config(2, 3, 1_000), ()).unwrap();
    std::thread::scope(|scope| {
        let first = scope.spawn(|| core.wait_until_idle(Duration::from_secs(2)));
        let second = scope.spawn(|| core.wait_until_idle(Duration::from_secs(2)));
        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
    });
    core.shutdown();
}

#[derive(Clone)]
struct BlockReplay {
    entered: crossbeam_channel::Sender<()>,
    release: crossbeam_channel::Receiver<()>,
}

impl checkpoints::ApplyWrapper<TestSnapshot, TestEvent> for BlockReplay {
    fn replay_event_batch(
        &mut self,
        batch: checkpoints::EventBatch<'_, u64, TestEvent>,
        inner: &mut checkpoints::ApplyInner<'_, TestSnapshot>,
    ) {
        inner.apply_event_batch(batch);
        self.entered.send(()).unwrap();
        self.release.recv().unwrap();
    }
}

#[test]
fn idle_wait_includes_after_apply_and_does_not_block_empty_queries() {
    let (entered, observed) = crossbeam_channel::unbounded();
    let (release, blocked) = crossbeam_channel::unbounded();
    let core = ConTime::start(config(2, 2, 1_000), BlockReplay { entered, release: blocked }).unwrap();
    core.advance_to(10).unwrap();
    let (tx, _rx) = crossbeam_channel::unbounded();
    core.send([event(1, 10, 1)], tx).unwrap();
    observed.recv_timeout(Duration::from_secs(2)).unwrap();
    let completed = core.wait_until_idle(Duration::from_millis(20));
    let (query_tx, query_rx) = crossbeam_channel::unbounded();
    core.send_query_at(10, [], query_tx).unwrap();
    let query = query_rx.recv_timeout(Duration::from_secs(2));
    release.send(()).unwrap();
    assert_eq!(completed, Err(contime_core::IdleError::Timeout));
    assert!(matches!(query, Err(crossbeam_channel::RecvTimeoutError::Disconnected)));
    core.wait_until_idle(Duration::from_secs(2)).unwrap();
    core.shutdown();
}

#[test]
fn late_buckets_and_boundary_updates_preserve_checkpoints() {
    for interval in [1, 2, 3, 10, 100] {
        for late_count in [1, 2, 10, 50] {
            let mut settings = config(1, 1, 50);
            settings.checkpoints.interval = interval;
            let core = ConTime::<TestEvent, TestSnapshot, ()>::start(settings, ()).unwrap();
            core.apply([event(1, 0, 1), event(2, 50, 1)]).unwrap();
            core.advance_to(50).unwrap();
            core.wait_until_idle(Duration::from_secs(2)).unwrap();
            assert!(core.errors().is_empty());
            let mut expected = 2;
            for round in 0..10u128 {
                // Late corrections at both existing buckets change the cadence
                // without changing the canonical final sum.
                let events = (0..late_count)
                    .flat_map(|i| [event(1000 + round * 1000 + i * 2, 0, 1), event(1001 + round * 1000 + i * 2, 50, 1)])
                    .collect::<Vec<_>>();
                core.apply(events).unwrap();
                core.wait_until_idle(Duration::from_secs(2)).unwrap();
                assert!(core.errors().is_empty());
                expected += late_count as u64 * 2;
                assert_eq!(
                    core.query_at(200, [7]).unwrap()[0].value,
                    expected,
                    "interval={interval} late_count={late_count} round={round}"
                );
            }
            core.advance_to(250).unwrap();
            core.wait_until_idle(Duration::from_secs(2)).unwrap();
            core.apply([event(99_000, 200, 1)]).unwrap();
            core.advance_to(300).unwrap();
            core.wait_until_idle(Duration::from_secs(2)).unwrap();
            assert!(core.errors().is_empty());
            let snapshots = core.query_at(300, [7]).unwrap();
            assert_eq!(snapshots[0].snapshot_id, 7);
            assert_eq!(snapshots[0].value, expected + 1);
            core.shutdown();
        }
    }
}

#[test]
fn uneven_late_buckets_preserve_checkpoint_suffix() {
    for interval in [2, 3, 5, 10] {
        let mut settings = config(1, 1, 30);
        settings.checkpoints.interval = interval;
        let core = ConTime::<TestEvent, TestSnapshot, ()>::start(settings, ()).unwrap();
        core.advance_to(30).unwrap();
        let mut seed = 17u64;
        let mut id = 1;
        for round in 0..500 {
            let mut batch = Vec::new();
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let size = 1 + seed % 17;
            for _ in 0..size {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                batch.push(event(id, seed % 30, 1));
                id += 1;
            }
            core.apply(batch).unwrap();
            core.wait_until_idle(Duration::from_secs(2)).unwrap();
            assert!(core.errors().is_empty(), "interval={interval} round={round}");
            assert_eq!(core.query_at(50, [7]).unwrap()[0].value, (id - 1) as u64);
        }
        core.shutdown();
    }
}

#[test]
fn repeated_pruning_and_updates_at_the_retained_boundary_preserve_state() {
    for interval in [1, 2, 3, 10, 100] {
        let mut settings = config(1, 1, 0);
        settings.checkpoints.interval = interval;
        let core = ConTime::<TestEvent, TestSnapshot, ()>::start(settings, ()).unwrap();
        let mut id = 1;
        for boundary in (0..1000).step_by(20) {
            for offset in [0, 10, 20, 1, 0, 19, 20, 0] {
                core.apply([event(id, boundary + offset, 1)])
                    .unwrap_or_else(|error| panic!("interval={interval} boundary={boundary} offset={offset} id={id}: {error:?}"));
                id += 1;
            }
            core.advance_to(boundary + 20).unwrap();
            core.wait_until_idle(Duration::from_secs(2)).unwrap();
            assert!(core.errors().is_empty(), "interval={interval} boundary={boundary}");
            core.apply([event(id, boundary + 20, 1)]).unwrap();
            core.wait_until_idle(Duration::from_secs(2)).unwrap();
            assert!(core.errors().is_empty(), "retained boundary correction interval={interval} boundary={boundary}");
            assert_eq!(core.query_at(boundary + 20, [7]).unwrap()[0].value, id as u64);
            id += 1;
        }
        core.shutdown();
    }
}
