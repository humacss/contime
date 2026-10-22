//! Historical queries, late arrivals, and retained duplicate event IDs.
//!
//! Run `cargo run -p contime --example ordered_values` from the workspace.

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

impl contime::AdvanceTime for Time {
    fn saturating_sub(&self, retention: &Self) -> Self {
        Self(self.0.saturating_sub(retention.0))
    }
}

#[derive(Hash)]
struct ReceiveValue {
    counter: u64,
    time: u64,
    value: i64,
}

fn values_id(counter: u64) -> u128 {
    // The same snapshot type identifier and counter key select the same history.
    u128::from(contentid::local_typed_content_id_u64("OrderedValues", &counter))
}

impl Event for ReceiveValue {
    type Time = Time;

    fn time(&self) -> Time {
        Time(self.time)
    }
}

impl Input for ReceiveValue {
    fn event_id(&self) -> u128 {
        u128::from(contentid::local_typed_content_id_u64("ReceiveValue", self))
    }

    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        emit(values_id(self.counter));
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

fn input(counter: u64, time: u64, value: i64) -> ReceiveValue {
    ReceiveValue { counter, time, value }
}

fn insert(history: &ConTime<ReceiveValue, OrderedValues, ()>, inputs: impl IntoIterator<Item = ReceiveValue>) -> Result<(), String> {
    let (responses, inserted) = crossbeam_channel::unbounded();
    history.send(inputs, responses).map_err(|error| format!("{error:?}"))?;
    // Closure acknowledges insertion on every route, not completed live replay.
    let rejections = inserted.iter().collect::<Vec<_>>();
    if !rejections.is_empty() {
        return Err(format!("unexpected rejections: {rejections:?}"));
    }
    Ok(())
}

fn main() -> Result<(), String> {
    let config = ConTimeConfig {
        router_count: 1,
        worker_count: 1,
        placement: Placement::default(),
        pruning_interval: Duration::from_millis(100),
        history_retention: Time(100),
        checkpoints: CheckpointConfig { interval: 2 },
    };
    let history = ConTime::<ReceiveValue, OrderedValues, ()>::start(config, ()).map_err(|error| format!("{error:?}"))?;
    history.advance_to(Time(10)).map_err(|error| format!("{error:?}"))?;
    let counter = 7;
    insert(&history, [input(counter, 5, 50), input(counter, 10, 100)])?;
    let before = history.query_at(Time(11), [values_id(counter)]).map_err(|error| format!("{error:?}"))?;
    assert_eq!(before[0].values, [50, 100]);
    println!("Before the late event: {:?}", before[0].values);

    // Rebuilding the first event produces the same ID and does not count it twice.
    insert(&history, [input(counter, 7, 70), input(counter, 5, 50)])?;
    for (time, expected) in [(6, vec![50]), (8, vec![50, 70]), (11, vec![50, 70, 100])] {
        let snapshots = history.query_at(Time(time), [values_id(counter)]).map_err(|error| format!("{error:?}"))?;
        assert_eq!(snapshots[0].values, expected);
        println!("Values visible at t={time}: {:?}", snapshots[0].values);
    }
    assert!(history.errors().is_empty());
    history.shutdown();
    Ok(())
}
