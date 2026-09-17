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
fn late_buckets_and_boundary_updates_preserve_checkpoints() {
    for interval in [1, 2, 3, 10, 100] {
        for late_count in [1, 2, 10, 50] {
            let mut settings = config(1, 1, 0);
            settings.checkpoints.interval = interval;
            let core = ConTime::<TestEvent, TestSnapshot, ()>::start(settings, ()).unwrap();
            assert!(core.apply([event(1, 0, 1), event(2, 50, 1)]).unwrap().is_empty());
            let mut expected = 2;
            for round in 0..10u128 {
                // Late corrections at both existing buckets change the cadence
                // without changing the canonical final sum.
                let events = (0..late_count)
                    .flat_map(|i| [event(1000 + round * 1000 + i * 2, 0, 1), event(1001 + round * 1000 + i * 2, 50, 1)])
                    .collect::<Vec<_>>();
                assert!(core.apply(events).unwrap().is_empty());
                expected += late_count as u64 * 2;
                assert_eq!(
                    core.query_at(200, [7]).unwrap()[0].value,
                    expected,
                    "interval={interval} late_count={late_count} round={round}"
                );
            }
            core.advance_to(200).unwrap();
            assert!(core.apply([event(99_000, 200, 1)]).unwrap().is_empty());
            core.advance_to(250).unwrap();
            let snapshots = core.query_at(250, [7]).unwrap();
            assert_eq!(snapshots[0].snapshot_id, 7);
            assert_eq!(snapshots[0].value, expected + 1);
            core.shutdown();
        }
    }
}

#[test]
fn uneven_late_buckets_preserve_checkpoint_suffix() {
    for interval in [2, 3, 5, 10] {
        let mut settings = config(1, 1, 0);
        settings.checkpoints.interval = interval;
        let core = ConTime::<TestEvent, TestSnapshot, ()>::start(settings, ()).unwrap();
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
            assert!(core.apply(batch).unwrap().is_empty(), "interval={interval} round={round}");
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
                assert!(core
                    .apply([event(id, boundary + offset, 1)])
                    .unwrap_or_else(|error| panic!("interval={interval} boundary={boundary} offset={offset} id={id}: {error:?}"))
                    .is_empty());
                id += 1;
            }
            core.advance_to(boundary + 20).unwrap();
            assert!(core.apply([event(id, boundary + 20, 1)]).unwrap().is_empty());
            assert_eq!(core.query_at(boundary + 20, [7]).unwrap()[0].value, id as u64);
            id += 1;
        }
        core.shutdown();
    }
}
