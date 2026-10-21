//! Historical queries and late arrivals through the current Core pipeline.
//!
//! Run `cargo run -p contime-core --example ordered_values` from the workspace.

use std::time::Duration;

use contime::checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Event, Snapshot, Timestamp};
use contime::{ConTime, ConTimeConfig, Input, Placement};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
struct Time(u64);

impl Timestamp for Time {
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}

impl contime_worker::AdvanceTime for Time {
    fn saturating_sub(&self, retention: &Self) -> Self {
        Self(self.0.saturating_sub(retention.0))
    }
}

struct ReceiveValue {
    snapshot_id: u128,
    event_id: u128,
    time: Time,
    value: i64,
}

impl Event for ReceiveValue {
    type Time = Time;

    fn time(&self) -> Time {
        self.time
    }
}

impl Input for ReceiveValue {
    fn event_id(&self) -> u128 {
        self.event_id
    }

    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        emit(self.snapshot_id);
    }
}

#[derive(Clone, Default)]
struct OrderedValues {
    time: Time,
    values: Vec<i64>,
}

impl Snapshot for OrderedValues {
    type Time = Time;

    fn time(&self) -> &Time {
        &self.time
    }

    fn set_time(&mut self, time: Time) {
        self.time = time;
    }
}

impl ApplyEvents<ReceiveValue> for OrderedValues {
    fn create(_snapshot_id: u128, _first_event: &ReceiveValue) -> Self {
        Self::default()
    }

    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Time, ReceiveValue>) {
        self.values.extend(batch.events.map(|event| event.value));
    }
}

fn input(event_id: u128, time: u64, value: i64) -> ReceiveValue {
    ReceiveValue { snapshot_id: 1, event_id, time: Time(time), value }
}

fn main() -> Result<(), String> {
    let config = ConTimeConfig {
        router_count: 1,
        worker_count: 1,
        placement: Placement::default(),
        pruning_interval: Duration::from_millis(100),
        history_retention: Time(100),
        worker: contime_worker::WorkerConfig {
            maximum_dirty_age: Duration::from_micros(100),
            replays_per_receive: 1,
            deadline_compaction_minimum: 1_024,
            deadline_compaction_multiplier: 2,
        },
        checkpoints: CheckpointConfig { interval: 2 },
    };
    let core = ConTime::<ReceiveValue, OrderedValues, ()>::start(config, ()).map_err(|error| format!("{error:?}"))?;
    core.apply([input(100, 5, 50), input(101, 10, 100)]).map_err(|error| format!("{error:?}"))?;
    core.wait_until_idle(Duration::from_secs(2)).map_err(|error| format!("{error:?}"))?;
    let before = core.query_at(Time(11), [1]).map_err(|error| format!("{error:?}"))?;
    assert_eq!(before[0].values, [50, 100]);
    println!("Before the late event: {:?}", before[0].values);

    core.apply([input(102, 7, 70)]).map_err(|error| format!("{error:?}"))?;
    core.wait_until_idle(Duration::from_secs(2)).map_err(|error| format!("{error:?}"))?;
    for (time, expected) in [(6, vec![50]), (8, vec![50, 70]), (11, vec![50, 70, 100])] {
        let snapshots = core.query_at(Time(time), [1]).map_err(|error| format!("{error:?}"))?;
        assert_eq!(snapshots[0].values, expected);
        println!("Values visible at t={time}: {:?}", snapshots[0].values);
    }
    assert!(core.errors().is_empty());
    core.shutdown();
    Ok(())
}
