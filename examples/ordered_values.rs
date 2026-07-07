//! Runnable onboarding example for `contime`.
//!
//! Run it from the repository root with:
//!
//! ```bash
//! cargo run --example ordered_values
//! ```
//!
//! This example is intentionally small, but it demonstrates the two core ideas a new user
//! usually needs to see first:
//!
//! 1. Queries are historical: you ask for the state of one snapshot at a chosen time.
//! 2. Late events are handled by replaying history in event-time order, so later queries
//!    reflect the corrected historical state.
//!
//! The domain model here is deliberately simple:
//!
//! - A snapshot stores a `Vec<i64>` of values it has received so far.
//! - An event contributes one new value at one event time.
//! - Applying an event only appends to the vector.
//!
//! There is no manual sorting logic anywhere in the snapshot implementation. The ordered
//! result comes from `contime` replaying events chronologically when it reconstructs state.
//!
//! The example timeline is:
//!
//! - Apply value `50` at `t=5`
//! - Apply value `100` at `t=10`
//! - Query at `t=11`, which initially yields `[50, 100]`
//! - Apply a late value `70` at `t=7`
//! - Re-query historical points
//!
//! After the late event arrives, querying at `t=11` yields `[50, 70, 100]`, even though the
//! event with value `70` was applied last in wall-clock order.

use contime::{ApplyBatch, ApplyEvents, Event, Snapshot, SnapshotEvent};

/// Point-in-time state for one logical stream of received values.
///
/// `contime` materializes this snapshot at arbitrary query times by replaying all earlier
/// events in chronological order.
#[derive(Clone, Debug, PartialEq, Eq)]
struct OrderedValuesSnapshot {
    /// Logical entity id. One snapshot history exists per id.
    id: u128,
    /// Time attached to the reconstructed snapshot returned by a query.
    time: i64,
    /// Values visible at this point in event-time history.
    values: Vec<i64>,
}

/// One input event that contributes a value to a snapshot history.
///
/// `event_id` is the stable identity used for ordering and duplicate detection, while `time`
/// is the historical point where the value should appear.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ReceiveValue {
    snapshot_id: u128,
    time: i64,
    event_id: u128,
    value: i64,
}

impl Snapshot for OrderedValuesSnapshot {
    type Event = ReceiveValue;

    /// Snapshot id used by `contime` to select the correct history lane.
    fn id(&self) -> u128 {
        self.id
    }

    /// Time attached to the materialized snapshot.
    ///
    /// When returned from a query, this is the query time rather than the time of the last
    /// event that was replayed.
    fn time(&self) -> i64 {
        self.time
    }

    /// `contime` updates the query result to the exact query time after replay.
    fn set_time(&mut self, time: i64) {
        self.time = time;
    }

    /// Conservative memory estimate used by `contime`'s memory budgeting.
    fn conservative_size(&self) -> u64 {
        16 + 8 + (self.values.len() * 8) as u64
    }

    /// Creates the initial snapshot state when the first event for a snapshot id arrives.
    ///
    /// The vector starts empty; replay then appends values in event-time order.
    fn from_event(event: &Self::Event) -> Self {
        Self { id: event.snapshot_id, time: event.time, values: Vec::new() }
    }
}

impl Event for ReceiveValue {
    /// Event id used for ordering and duplicate detection.
    fn id(&self) -> u128 {
        self.event_id
    }

    /// Historical time where this value should be inserted.
    fn time(&self) -> i64 {
        self.time
    }

    /// Conservative memory estimate used by `contime`'s memory budgeting.
    fn conservative_size(&self) -> u64 {
        16 + 8 + 16 + 8
    }
}

impl SnapshotEvent<OrderedValuesSnapshot> for ReceiveValue {
    /// Snapshot history targeted by this event.
    fn snapshot_id(&self) -> u128 {
        self.snapshot_id
    }
}

impl ApplyEvents for OrderedValuesSnapshot {
    /// Applies one event to one replay step.
    ///
    /// The example intentionally avoids any custom insertion or sorting logic. Values end up
    /// ordered because `contime` replays events in event-time order.
    fn apply_events(&mut self, batch: ApplyBatch<'_, Self::Event>) {
        self.id = batch.snapshot_id;
        for event in batch.events.iter().copied() {
            self.values.push(event.value);
        }
        self.time = batch.time;
    }
}

// Generate the lane enums and a typed `Contime` alias for this single-snapshot example.
contime::lanes! {
    mod ordered_values_lanes;
    snapshots [OrderedValuesSnapshot];
    routes [
        ReceiveValue => [OrderedValuesSnapshot],
    ];
}

/// Small constructor helper to keep the timeline in `main` easy to scan.
fn receive_value(snapshot_id: u128, time: i64, event_id: u128, value: i64) -> ReceiveValue {
    ReceiveValue { snapshot_id, time, event_id, value }
}

/// Prints one reconstructed snapshot in a compact form.
fn show_snapshot(label: &str, snapshot: &OrderedValuesSnapshot) {
    println!("{label}: t={} values={:?}", snapshot.time, snapshot.values);
}

fn query_snapshot(contime: &ordered_values_lanes::Contime, time: i64, snapshot_id: u128) -> OrderedValuesSnapshot {
    contime.query_at(time, &[snapshot_id]).expect("query should succeed").pop().flatten().expect("snapshot should exist").into()
}

fn main() {
    // One worker is enough for this example. The memory budget only needs to be large enough
    // for a handful of small events and snapshots.
    let contime = ordered_values_lanes::Contime::new(1, 4_096);

    println!("Building continuous-time history for snapshot 1.");
    println!("Query times are inclusive, so querying at an event time includes that event.");

    // Start with two in-order events so the baseline history is easy to reason about.
    println!("Applying event at t=5 with value 50.");
    contime.apply_events([receive_value(1, 5, 100, 50)]).expect("first event should apply");

    println!("Applying event at t=10 with value 100.");
    contime.apply_events([receive_value(1, 10, 101, 100)]).expect("second event should apply");

    // Query after both events. At this point the observed history is [50, 100].
    let before_late_event = query_snapshot(&contime, 11, 1);
    assert_eq!(before_late_event.values, vec![50, 100]);
    show_snapshot("Before the late event", &before_late_event);

    // Apply a late event whose event time belongs between the two earlier events.
    // Wall-clock arrival order is now different from event-time order.
    println!("Applying a late event at t=7 with value 70.");
    contime.apply_events([receive_value(1, 7, 102, 70)]).expect("late event should apply");

    // Query before the late event's time: only the first value is visible.
    let at_6 = query_snapshot(&contime, 6, 1);
    assert_eq!(at_6.values, vec![50]);
    show_snapshot("Values visible at t=6", &at_6);

    // Query just after the late event: the new value appears between the two original events.
    let at_8 = query_snapshot(&contime, 8, 1);
    assert_eq!(at_8.values, vec![50, 70]);
    show_snapshot("Values visible at t=8", &at_8);

    // Query the later time again. The final history is now chronologically ordered even
    // though value 70 was the last event applied in real time.
    let at_11 = query_snapshot(&contime, 11, 1);
    assert_eq!(at_11.values, vec![50, 70, 100]);
    show_snapshot("Values visible at t=11 after replay", &at_11);

    println!("The late event is inserted into the correct historical position without custom sorting.");
}
