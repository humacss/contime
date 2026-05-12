use std::collections::BTreeMap;
use std::ops::Bound;

use crate::{ApplyEvent, ContimeKey, Event, Snapshot};

/// Replays events from `start_snapshot` forward (starting after `start_bound`) and creates
/// checkpoints at every `checkpoint_interval` events. Returns the bytes_delta from new checkpoints.
pub fn replay_and_checkpoint<S: Snapshot>(
    start_snapshot: &S,
    start_bound: Bound<&ContimeKey>,
    checkpoints: &mut BTreeMap<ContimeKey, S>,
    events: &BTreeMap<ContimeKey, S::Event>,
    checkpoint_interval: usize,
) -> i64 {
    let mut bytes_delta: i64 = 0;
    let mut snapshot = start_snapshot.clone();

    let mut count = 0;
    for (key, event) in events.range((start_bound, Bound::Unbounded)) {
        event.apply_to(&mut snapshot);
        snapshot.set_time(event.time());
        count += 1;

        if count % checkpoint_interval == 0 {
            bytes_delta += snapshot.conservative_size() as i64;
            checkpoints.insert(key.clone(), snapshot.clone());
        }
    }

    bytes_delta
}
