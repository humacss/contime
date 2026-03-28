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
//! 2. Late events are handled by replaying history in event-time order, which can trigger
//!    reconciliation for readers that already queried a later point in time.
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
//! - Observe reconciliation and re-query historical points
//!
//! After the late event arrives, querying at `t=11` yields `[50, 70, 100]`, even though the
//! event with value `70` was applied last in wall-clock order.

use std::time::Duration;

use contime::{ApplyEvent, Event, Snapshot};

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

impl ApplyEvent<OrderedValuesSnapshot> for ReceiveValue {
    /// Snapshot history targeted by this event.
    fn snapshot_id(&self) -> u128 {
        self.snapshot_id
    }

    /// Applies one event to one replay step.
    ///
    /// The example intentionally avoids any custom insertion or sorting logic. Values end up
    /// ordered because `contime` replays events in event-time order.
    fn apply_to(&self, snapshot: &mut OrderedValuesSnapshot) {
        snapshot.values.push(self.value);
    }
}

// Generate the lane enums and a typed `Contime` alias for this single-snapshot example.
contime::contime! {
    mod ordered_values_lanes;
    OrderedValuesSnapshot {
        ReceiveValue,
    }
}

/// Small constructor helper to keep the timeline in `main` easy to scan.
fn receive_value(snapshot_id: u128, time: i64, event_id: u128, value: i64) -> ReceiveValue {
    ReceiveValue { snapshot_id, time, event_id, value }
}

/// Prints one reconstructed snapshot in a compact form.
fn show_snapshot(label: &str, snapshot: &OrderedValuesSnapshot) {
    println!("{label}: t={} values={:?}", snapshot.time, snapshot.values);
}

fn main() {
    // One worker is enough for this example. The memory budget only needs to be large enough
    // for a handful of small events and snapshots.
    let contime = ordered_values_lanes::Contime::new(1, 4_096);

    println!("Building continuous-time history for snapshot 1.");
    println!("Query times are exclusive, so query one tick after the last event you want included.");

    // Start with two in-order events so the baseline history is easy to reason about.
    println!("Applying event at t=5 with value 50.");
    contime.apply_event(receive_value(1, 5, 100, 50)).expect("first event should apply");

    println!("Applying event at t=10 with value 100.");
    contime.apply_event(receive_value(1, 10, 101, 100)).expect("second event should apply");

    // Query after both events. At this point the observed history is [50, 100].
    // Keep the reconciliation receiver so we can detect later historical changes.
    let (before_late_event, reconciliation_rx) =
        contime.at::<OrderedValuesSnapshot>(11, 1).expect("snapshot should exist after initial events");
    assert_eq!(before_late_event.values, vec![50, 100]);
    show_snapshot("Before the late event", &before_late_event);

    // Apply a late event whose event time belongs between the two earlier events.
    // Wall-clock arrival order is now different from event-time order.
    println!("Applying a late event at t=7 with value 70.");
    contime.apply_event(receive_value(1, 7, 102, 70)).expect("late event should apply");

    // Because we already queried a later time, `contime` reports that the historical range
    // from t=7 through the previous latest event time must be reconsidered.
    let reconciliation = reconciliation_rx.recv_timeout(Duration::from_secs(1)).expect("late event should trigger reconciliation");
    assert_eq!(reconciliation.snapshot_id, 1);
    assert_eq!(reconciliation.from_time, 7);
    assert_eq!(reconciliation.to_time, 10);
    println!(
        "Reconciliation requested for snapshot {} from t={} to t={}.",
        reconciliation.snapshot_id, reconciliation.from_time, reconciliation.to_time
    );

    // Query before the late event's time: only the first value is visible.
    let (at_6, _) = contime.at::<OrderedValuesSnapshot>(6, 1).expect("query at t=6 should succeed");
    assert_eq!(at_6.values, vec![50]);
    show_snapshot("Values visible at t=6", &at_6);

    // Query just after the late event: the new value appears between the two original events.
    let (at_8, _) = contime.at::<OrderedValuesSnapshot>(8, 1).expect("query at t=8 should succeed");
    assert_eq!(at_8.values, vec![50, 70]);
    show_snapshot("Values visible at t=8", &at_8);

    // Query the later time again. The final history is now chronologically ordered even
    // though value 70 was the last event applied in real time.
    let (at_11, _) = contime.at::<OrderedValuesSnapshot>(11, 1).expect("query at t=11 should succeed");
    assert_eq!(at_11.values, vec![50, 70, 100]);
    show_snapshot("Values visible at t=11 after replay", &at_11);

    println!("The late event is inserted into the correct historical position without custom sorting.");
}
