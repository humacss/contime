use std::time::Duration;

use contime_core::checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Snapshot};
use contime_core::memory_tracking::ConservativeTrackedSize;
use contime_core::{ConTime, ConTimeConfig, Input};

struct Event {
    id: u128,
    time: u64,
    value: u64,
}

impl ConservativeTrackedSize for Event {
    fn conservative_tracked_size(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

impl Input for Event {
    type Time = u64;

    fn event_id(&self) -> u128 {
        self.id
    }

    fn time(&self) -> Self::Time {
        self.time
    }

    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        emit(7);
    }
}

#[derive(Clone, Default)]
struct State {
    time: u64,
    value: u64,
}

impl ConservativeTrackedSize for State {
    fn conservative_tracked_size(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

impl Snapshot for State {
    type Time = u64;

    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }
}

impl ApplyEvents<Event> for State {
    fn create(_snapshot_id: u128, _first_event: &Event) -> Self {
        Self::default()
    }

    fn apply_events(&mut self, batch: ApplyBatch<'_, Self::Time, Event>) {
        self.value += batch.events.iter().map(|event| event.value).sum::<u64>();
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
        checkpoints: CheckpointConfig { interval: 2 },
    }
}

#[test]
fn public_queries_return_historical_snapshots_and_owned_event_handles() {
    let contime = ConTime::<Event, State, ()>::start(config(), ()).unwrap();
    contime.apply([Event { id: 1, time: 10, value: 1 }, Event { id: 2, time: 20, value: 2 }, Event { id: 3, time: 30, value: 4 }]).unwrap();

    let snapshots = contime.query_at(20, [7]).unwrap();
    let events = contime.query_events_between(7, 10, 30).unwrap();

    assert_eq!((snapshots[0].time, snapshots[0].value), (20, 3));
    assert_eq!(events.iter().map(|event| event.event_id()).collect::<Vec<_>>(), vec![1, 2]);
    contime.shutdown();
}
