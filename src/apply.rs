
use std::collections::BTreeMap;
use std::ops::Bound;

use crate::{ApplyEvent, Event, Snapshot, ContimeKey};

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

pub fn apply_event_in_place<S: Snapshot>(
    new_event: S::Event,
    base_snapshot: &S,
    checkpoints: &mut BTreeMap<ContimeKey, S>,
    events: &mut BTreeMap<ContimeKey, S::Event>,
    checkpoint_interval: usize,
) -> i64 {
    let event_key = ContimeKey::from_event(&new_event);

    // Duplicate check
    if events.contains_key(&event_key) {
        return 0;
    }

    let mut bytes_delta = new_event.conservative_size() as i64;

    // Insert event
    events.insert(event_key.clone(), new_event);

    // Fast path: if this is the latest event, no existing checkpoints are stale
    let is_latest = events.range((Bound::Excluded(&event_key), Bound::Unbounded)).next().is_none();

    if is_latest {
        // Find the last checkpoint to replay from
        let last_checkpoint = checkpoints.iter().next_back();

        let (snapshot, start_bound) = match last_checkpoint {
            Some((key, checkpoint)) => (checkpoint.clone(), Bound::Excluded(key.clone())),
            None => (base_snapshot.clone(), Bound::Unbounded),
        };

        // Count events from start_bound to determine checkpoint eligibility
        let events_from_start: usize = events.range((
            match &start_bound {
                Bound::Excluded(k) => Bound::Excluded(k),
                _ => Bound::Unbounded,
            },
            Bound::Unbounded,
        )).count();

        // Only need to replay if we're at a checkpoint boundary
        if events_from_start % checkpoint_interval == 0 {
            // Replay from last checkpoint to create the new checkpoint
            let mut snap = snapshot;
            for (_k, ev) in events.range((
                match &start_bound {
                    Bound::Excluded(k) => Bound::Excluded(k),
                    _ => Bound::Unbounded,
                },
                Bound::Unbounded,
            )) {
                ev.apply_to(&mut snap);
                snap.set_time(ev.time());
            }
            bytes_delta += snap.conservative_size() as i64;
            checkpoints.insert(event_key, snap);
        }
    } else {
        // Slow path: out-of-order event, need to invalidate and replay

        // Find last valid checkpoint: the latest checkpoint whose key is before the new event
        let replay_start_key = checkpoints
            .range(..&event_key)
            .next_back()
            .map(|(k, _)| k.clone());

        // Remove stale checkpoints (at or after the new event key)
        let stale_keys: Vec<ContimeKey> = checkpoints
            .range(event_key..)
            .map(|(k, _)| k.clone())
            .collect();
        for key in &stale_keys {
            let removed = checkpoints.remove(key).expect("stale checkpoint key must exist");
            bytes_delta -= removed.conservative_size() as i64;
        }

        // Build starting snapshot
        let snapshot = match &replay_start_key {
            Some(key) => checkpoints.get(key).expect("replay start checkpoint must exist").clone(),
            None => base_snapshot.clone(),
        };

        let start_bound = match &replay_start_key {
            Some(key) => Bound::Excluded(key),
            None => Bound::Unbounded,
        };

        bytes_delta += replay_and_checkpoint(
            &snapshot,
            start_bound,
            checkpoints,
            events,
            checkpoint_interval,
        );
    }

    bytes_delta
}


#[cfg(test)]
mod tests {
    use super::*;

    use crate::{TestEvent, TestSnapshot};
    use rstest::rstest;

    type Value = i16;
    type Time = i64;
    type Sum = i32;
    type Items = Vec<i16>;
    const CHECKPOINT_INTERVAL: usize = 2;

    fn gen_event(time: Time, value: Value) -> TestEvent {
        let snapshot_id = 0;

        if value >= 0 {
            TestEvent::Positive(snapshot_id, time, time.try_into().unwrap(), value.abs() as u16)
        } else {
            TestEvent::Negative(snapshot_id, time, time.try_into().unwrap(), value.abs() as u16)
        }
    }

    fn base_snapshot() -> TestSnapshot {
        TestSnapshot { id: 0, time: 0, sum: 0, items: vec![] }
    }

    fn build_events(raw: Vec<(Time, Value)>) -> BTreeMap<ContimeKey, TestEvent> {
        let mut map = BTreeMap::new();
        for (time, value) in raw {
            let event = gen_event(time, value);
            let key = ContimeKey::from_event(&event);
            map.insert(key, event);
        }
        map
    }

    fn build_checkpoints(raw: Vec<(Time, Sum, Items)>) -> BTreeMap<ContimeKey, TestSnapshot> {
        let mut map = BTreeMap::new();
        for (time, sum, items) in raw {
            let snapshot = TestSnapshot { id: 0, time, sum, items };
            // Checkpoint key matches the event key that produced it.
            // In gen_event, event_id = time as u128.
            let key = ContimeKey { time, id: time as u128 };
            map.insert(key, snapshot);
        }
        map
    }

    #[rstest]
    #[case::empty(
        (1, 1),
        vec![],
        vec![],
        vec![],
        vec![(1, 1)],
    )]
    #[case::in_order(
        (2, 2),
        vec![],
        vec![(1, 1)],
        vec![(2, 3, vec![1, 2])],
        vec![(1, 1), (2, 2)],
    )]
    #[case::out_of_order(
        (1, 1),
        vec![],
        vec![(2, 2)],
        vec![(2, 3, vec![1, 2])],
        vec![(1, 1), (2, 2)],
    )]
    #[case::duplicate_event(
        (1, 1),
        vec![],
        vec![(1, 1)],
        vec![],
        vec![(1, 1)],
    )]
    fn test_apply_event_in_place(
        #[case] new_event: (Time, Value),
        #[case] initial_checkpoints: Vec<(Time, Sum, Items)>,
        #[case] initial_events: Vec<(Time, Value)>,
        #[case] expected_checkpoints: Vec<(Time, Sum, Items)>,
        #[case] expected_events: Vec<(Time, Value)>,
    ) {
        let base = base_snapshot();
        let mut checkpoints = build_checkpoints(initial_checkpoints);
        let mut events = build_events(initial_events.clone());

        let expected_events = build_events(expected_events);
        let expected_checkpoints = build_checkpoints(expected_checkpoints);

        let new_event = gen_event(new_event.0, new_event.1);

        apply_event_in_place::<TestSnapshot>(
            new_event,
            &base,
            &mut checkpoints,
            &mut events,
            CHECKPOINT_INTERVAL,
        );

        assert_eq!(expected_events, events);
        assert_eq!(expected_checkpoints, checkpoints);
    }
}
