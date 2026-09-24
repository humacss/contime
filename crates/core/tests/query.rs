use std::time::Duration;

use contime_core::checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Snapshot};
use contime_core::{ConTime, ConTimeConfig, Input};

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

struct Event {
    id: u128,
    time: Time,
    value: u64,
}

impl contime_checkpoints::Event for Event {
    type Time = Time;

    fn time(&self) -> Self::Time {
        self.time
    }
}
impl Input for Event {
    fn event_id(&self) -> u128 {
        self.id
    }
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        emit(7);
    }
}

#[derive(Clone, Default)]
struct State {
    time: Time,
    value: u64,
}

impl Snapshot for State {
    type Time = Time;

    fn time(&self) -> &Self::Time {
        &self.time
    }
    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }
}

impl ApplyEvents<Event> for State {
    fn create(_snapshot_id: u128, _first_event: &Event) -> Self {
        Self::default()
    }

    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Self::Time, Event>) {
        self.value += batch.events.map(|event| event.value).sum::<u64>();
    }
}

fn config() -> ConTimeConfig<Time> {
    ConTimeConfig {
        router_count: 2,
        worker_count: 4,
        placement: contime_router::Placement::default(),
        pruning_interval: std::time::Duration::from_millis(100),

        history_retention: Time(0),
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
    contime
        .apply([
            Event { id: 1, time: Time(10), value: 1 },
            Event { id: 2, time: Time(20), value: 2 },
            Event { id: 3, time: Time(30), value: 4 },
        ])
        .unwrap();
    contime.wait_until_idle(Duration::from_secs(2)).unwrap();
    assert!(contime.errors().is_empty());

    let snapshots = contime.query_at(Time(20), [7]).unwrap();
    let events = contime.query_events_between(7, Time(10), Time(30)).unwrap();

    assert_eq!((snapshots[0].time, snapshots[0].value), (Time(20), 3));
    assert_eq!(events.iter().map(|event| event.event_id()).collect::<Vec<_>>(), vec![1, 2]);
    contime.shutdown();
}
