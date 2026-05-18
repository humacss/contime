use std::collections::BTreeMap;
use std::ops::Bound;

use crate::{ApplyEvents, ContimeKey, Snapshot};

/// Replays events from `start_snapshot` forward (starting after `start_bound`) and creates
/// checkpoints at every `checkpoint_interval` events. Returns the bytes_delta from new checkpoints.
pub fn replay_and_checkpoint<S: Snapshot + ApplyEvents<()>>(
    start_snapshot: &S,
    start_bound: Bound<&ContimeKey>,
    checkpoints: &mut BTreeMap<ContimeKey, S>,
    events: &BTreeMap<ContimeKey, S::Event>,
    checkpoint_interval: usize,
) -> i64 {
    let mut bytes_delta: i64 = 0;
    let mut snapshot = start_snapshot.clone();

    let mut count = 0;
    let replay_events = events.range((start_bound, Bound::Unbounded)).map(|(key, event)| (key.clone(), event.clone())).collect::<Vec<_>>();
    let mut index = 0usize;
    while index < replay_events.len() {
        let bucket_time = replay_events[index].0.time;
        let mut bucket = Vec::new();
        let mut bucket_last_key = replay_events[index].0.clone();

        while index < replay_events.len() && replay_events[index].0.time == bucket_time {
            bucket_last_key = replay_events[index].0.clone();
            bucket.push(replay_events[index].1.clone());
            index += 1;
        }

        snapshot.apply_events(bucket_time, &bucket);
        snapshot.set_time(bucket_time);
        count += bucket.len();

        if checkpoint_interval != 0 && count % checkpoint_interval == 0 {
            bytes_delta += snapshot.conservative_size() as i64;
            checkpoints.insert(bucket_last_key, snapshot.clone());
        }
    }

    bytes_delta
}
