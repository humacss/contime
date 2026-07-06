use std::collections::{BTreeMap, VecDeque};
use std::ops::Bound;

use std::convert::Infallible;

use crate::{ApplyBatch, ApplyDecision, ApplyEvents, ApplyInner, ApplyWrapper, ContimeKey, Snapshot};

use super::history::LocalSnapshotHistory;

pub(super) fn first_key_at_time(time: i64) -> ContimeKey {
    ContimeKey { time, id: u128::MIN }
}

pub(super) fn last_key_at_time(time: i64) -> ContimeKey {
    ContimeKey { time, id: u128::MAX }
}

pub(super) fn checkpoint_partition_before<S>(checkpoints: &VecDeque<(ContimeKey, S)>, boundary: &ContimeKey) -> usize {
    let mut low = 0;
    let mut high = checkpoints.len();
    while low < high {
        let mid = (low + high) / 2;
        if checkpoints[mid].0 < *boundary {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    low
}

pub(super) fn latest_checkpoint_before_index<S>(checkpoints: &VecDeque<(ContimeKey, S)>, boundary: &ContimeKey) -> Option<usize> {
    let index = checkpoint_partition_before(checkpoints, boundary);
    (index > 0).then(|| index - 1)
}

pub(super) fn latest_checkpoint_before<'a, S>(
    checkpoints: &'a VecDeque<(ContimeKey, S)>,
    boundary: &ContimeKey,
) -> Option<(&'a ContimeKey, &'a S)> {
    latest_checkpoint_before_index(checkpoints, boundary).map(|index| {
        let (key, checkpoint) = &checkpoints[index];
        (key, checkpoint)
    })
}

pub(super) fn latest_checkpoint_at_or_before<'a, S>(
    checkpoints: &'a VecDeque<(ContimeKey, S)>,
    boundary: &ContimeKey,
) -> Option<(&'a ContimeKey, &'a S)> {
    let mut low = 0;
    let mut high = checkpoints.len();
    while low < high {
        let mid = (low + high) / 2;
        if checkpoints[mid].0 <= *boundary {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    let index = low;
    (index > 0).then(|| {
        let (key, checkpoint) = &checkpoints[index - 1];
        (key, checkpoint)
    })
}

pub(super) fn push_checkpoint<S>(checkpoints: &mut VecDeque<(ContimeKey, S)>, key: ContimeKey, checkpoint: S) -> i64
where
    S: Snapshot,
{
    let bytes_delta = checkpoint.conservative_size() as i64;
    checkpoints.push_back((key, checkpoint));
    bytes_delta
}

pub(super) fn drain_checkpoints_from<S>(checkpoints: &mut VecDeque<(ContimeKey, S)>, start: usize) -> i64
where
    S: Snapshot,
{
    let mut bytes_delta = 0;
    for (_key, removed) in checkpoints.drain(start..) {
        bytes_delta -= removed.conservative_size() as i64;
    }
    bytes_delta
}

pub(super) struct CheckpointForApply<S>
where
    S: Snapshot,
{
    snapshot: S,
    start: Bound<ContimeKey>,
    end: Bound<ContimeKey>,
    stale_start: Option<usize>,
    preserve_previous_tip: bool,
    first_changed_time: i64,
    exact_event_key: Option<ContimeKey>,
    bytes_delta: i64,
}

pub(super) struct AppliedCheckpoint<S>
where
    S: Snapshot,
{
    stale_start: Option<usize>,
    bytes_delta: i64,
    final_key: Option<ContimeKey>,
    final_snapshot: S,
    materialized_checkpoints: Vec<(ContimeKey, S)>,
}

pub(super) fn get_checkpoint_for_apply<S>(
    history: &mut LocalSnapshotHistory<S>,
    earliest_changed_time: i64,
    latest_event_key_before_apply: Option<ContimeKey>,
    single_changed_event_key: Option<ContimeKey>,
) -> CheckpointForApply<S>
where
    S: Snapshot + ApplyEvents + 'static,
{
    let preserve_previous_tip = latest_event_key_before_apply.as_ref().is_none_or(|latest| earliest_changed_time > latest.time);

    if preserve_previous_tip {
        if let Some(single_changed_event_key) = single_changed_event_key {
            return get_latest_event_checkpoint_for_apply(history, latest_event_key_before_apply, single_changed_event_key);
        }
    }

    get_recomputed_checkpoint_for_apply(history, earliest_changed_time, preserve_previous_tip)
}

pub(super) fn apply_events_to_checkpoint<S, C>(
    history: &LocalSnapshotHistory<S>,
    mut checkpoint: CheckpointForApply<S>,
    context: &mut C,
) -> Result<AppliedCheckpoint<S>, C::Error>
where
    S: Snapshot + ApplyEvents + 'static,
    C: ApplyWrapper<S>,
{
    let mut event_count = 0usize;
    let mut materialized_checkpoints = Vec::new();
    let mut stored_first_changed_checkpoint = false;

    if let Some(exact_event_key) = checkpoint.exact_event_key.take() {
        let event = history.events.get(&exact_event_key).expect("changed event must be stored before checkpoint apply");
        let bucket = [event];
        let batch = ApplyBatch { snapshot_id: history.snapshot_id, time: exact_event_key.time, events: &bucket };
        context.apply_event_batch_wrapper(&mut checkpoint.snapshot, batch, ApplyInner::default())?;
        return Ok(AppliedCheckpoint {
            stale_start: checkpoint.stale_start,
            bytes_delta: checkpoint.bytes_delta,
            final_key: Some(exact_event_key),
            final_snapshot: checkpoint.snapshot,
            materialized_checkpoints,
        });
    }

    apply_event_buckets::<S, C::Error, _>(
        history.snapshot_id,
        &history.events,
        checkpoint.start.clone(),
        checkpoint.end.clone(),
        |bucket_last_key, bucket_len, batch| {
            if context.apply_event_batch_wrapper(&mut checkpoint.snapshot, batch, ApplyInner::default())? == ApplyDecision::EarlyExit {
                return Ok(false);
            }
            event_count += bucket_len;

            if !checkpoint.preserve_previous_tip && !stored_first_changed_checkpoint && batch.time >= checkpoint.first_changed_time {
                materialized_checkpoints.push((bucket_last_key.clone(), checkpoint.snapshot.clone()));
                stored_first_changed_checkpoint = true;
            }

            if checkpoint.preserve_previous_tip && checkpoint.end == Bound::Unbounded && history.checkpoint_interval != 0 {
                if event_count % history.checkpoint_interval == 0 {
                    materialized_checkpoints.push((bucket_last_key.clone(), checkpoint.snapshot.clone()));
                }
            }

            Ok(true)
        },
    )?;

    Ok(AppliedCheckpoint {
        stale_start: checkpoint.stale_start,
        bytes_delta: checkpoint.bytes_delta,
        final_key: history.events.keys().next_back().cloned(),
        final_snapshot: checkpoint.snapshot,
        materialized_checkpoints,
    })
}

pub(super) fn commit_applied_checkpoint<S>(history: &mut LocalSnapshotHistory<S>, applied_checkpoint: AppliedCheckpoint<S>) -> i64
where
    S: Snapshot + ApplyEvents + 'static,
{
    let mut bytes_delta = applied_checkpoint.bytes_delta;

    if let Some(stale_start) = applied_checkpoint.stale_start {
        bytes_delta += drain_checkpoints_from(&mut history.checkpoints, stale_start);
    }

    for (key, checkpoint) in applied_checkpoint.materialized_checkpoints {
        bytes_delta += push_checkpoint(&mut history.checkpoints, key, checkpoint);
    }

    if let Some(latest_key) = applied_checkpoint.final_key {
        if history.checkpoints.back().map(|(key, _checkpoint)| key) != Some(&latest_key) {
            bytes_delta += push_checkpoint(&mut history.checkpoints, latest_key, applied_checkpoint.final_snapshot);
        }
    }

    bytes_delta
}

fn get_latest_event_checkpoint_for_apply<S>(
    history: &mut LocalSnapshotHistory<S>,
    latest_event_key_before_apply: Option<ContimeKey>,
    single_changed_event_key: ContimeKey,
) -> CheckpointForApply<S>
where
    S: Snapshot + ApplyEvents + 'static,
{
    let previous_event_count = history.events.len().saturating_sub(1);
    let previous_tip_is_cadence =
        latest_event_key_before_apply.as_ref().is_some_and(|_key| is_event_count_cadence(history, previous_event_count));

    let mut bytes_delta = 0;
    let snapshot = if let Some(previous_key) = latest_event_key_before_apply.as_ref().filter(|_key| !previous_tip_is_cadence) {
        match history.checkpoints.back().map(|(key, _checkpoint)| key == previous_key).unwrap_or(false) {
            true => {
                let (_key, previous_tip) = history.checkpoints.pop_back().expect("previous tip checkpoint must exist");
                bytes_delta -= previous_tip.conservative_size() as i64;
                previous_tip
            }
            false => latest_checkpoint_or_base(history),
        }
    } else {
        latest_checkpoint_or_base(history)
    };

    CheckpointForApply {
        snapshot,
        start: Bound::Unbounded,
        end: Bound::Unbounded,
        stale_start: None,
        preserve_previous_tip: true,
        first_changed_time: i64::MAX,
        exact_event_key: Some(single_changed_event_key),
        bytes_delta,
    }
}

fn get_recomputed_checkpoint_for_apply<S>(
    history: &LocalSnapshotHistory<S>,
    time: i64,
    preserve_previous_tip: bool,
) -> CheckpointForApply<S>
where
    S: Snapshot + ApplyEvents + 'static,
{
    let mut recompute_boundary = first_key_at_time(time);
    if !preserve_previous_tip {
        if let Some((key, _checkpoint)) = latest_checkpoint_before(&history.checkpoints, &recompute_boundary) {
            if !checkpoint_key_is_cadence(history, key) {
                recompute_boundary = key.clone();
            }
        }
    }

    let checkpoint_index = latest_checkpoint_before_index(&history.checkpoints, &recompute_boundary);
    let remove_recompute_checkpoint =
        preserve_previous_tip && checkpoint_index.is_some_and(|index| !checkpoint_key_is_cadence(history, &history.checkpoints[index].0));
    let (snapshot, start) = match checkpoint_index {
        Some(index) => {
            let (key, checkpoint) = &history.checkpoints[index];
            (checkpoint.clone(), Bound::Excluded(key.clone()))
        }
        None => (history.base_snapshot.clone(), Bound::Unbounded),
    };

    let stale_start = if remove_recompute_checkpoint {
        checkpoint_index
    } else {
        Some(checkpoint_partition_before(&history.checkpoints, &recompute_boundary))
    };

    CheckpointForApply {
        snapshot,
        start,
        end: Bound::Unbounded,
        stale_start,
        preserve_previous_tip,
        first_changed_time: time,
        exact_event_key: None,
        bytes_delta: 0,
    }
}

fn latest_checkpoint_or_base<S>(history: &LocalSnapshotHistory<S>) -> S
where
    S: Snapshot,
{
    history.checkpoints.back().map(|(_key, checkpoint)| checkpoint.clone()).unwrap_or_else(|| history.base_snapshot.clone())
}

fn is_event_count_cadence<S>(history: &LocalSnapshotHistory<S>, event_count: usize) -> bool
where
    S: Snapshot,
{
    history.checkpoint_interval != 0 && event_count != 0 && event_count % history.checkpoint_interval == 0
}

fn checkpoint_key_is_cadence<S>(history: &LocalSnapshotHistory<S>, checkpoint_key: &ContimeKey) -> bool
where
    S: Snapshot,
{
    if history.checkpoint_interval == 0 || history.checkpoints.back().map(|(key, _checkpoint)| key) != Some(checkpoint_key) {
        return true;
    }

    is_event_count_cadence(history, history.events.len())
}

pub(super) fn apply_event_buckets<S, E, F>(
    snapshot_id: u128,
    events: &BTreeMap<ContimeKey, S::Event>,
    start: Bound<ContimeKey>,
    end: Bound<ContimeKey>,
    mut apply_bucket: F,
) -> Result<(), E>
where
    S: Snapshot + ApplyEvents + 'static,
    F: FnMut(&ContimeKey, usize, ApplyBatch<'_, S::Event>) -> Result<bool, E>,
{
    let mut iter = events.range((start, end)).peekable();
    while let Some((first_key, first_event)) = iter.next() {
        let bucket_time = first_key.time;
        let mut bucket_last_key = first_key;

        if iter.peek().is_none_or(|(next_key, _next_event)| next_key.time != bucket_time) {
            let bucket = [first_event];
            let batch = ApplyBatch { snapshot_id, time: bucket_time, events: &bucket };
            if !apply_bucket(bucket_last_key, 1, batch)? {
                break;
            }
            continue;
        }

        let (second_key, second_event) = iter.next().expect("same-time bucket second event must exist");
        bucket_last_key = second_key;

        if iter.peek().is_none_or(|(next_key, _next_event)| next_key.time != bucket_time) {
            let bucket = [first_event, second_event];
            let batch = ApplyBatch { snapshot_id, time: bucket_time, events: &bucket };
            if !apply_bucket(bucket_last_key, 2, batch)? {
                break;
            }
            continue;
        }

        let remaining_same_time = iter.clone().take_while(|(key, _event)| key.time == bucket_time).count();
        let mut bucket = Vec::with_capacity(2 + remaining_same_time);
        bucket.push(first_event);
        bucket.push(second_event);

        while let Some((next_key, _next_event)) = iter.peek() {
            if next_key.time != bucket_time {
                break;
            }
            let (key, event) = iter.next().expect("peeked event must exist");
            bucket_last_key = key;
            bucket.push(event);
        }

        let bucket_len = bucket.len();
        let batch = ApplyBatch { snapshot_id, time: bucket_time, events: &bucket };
        if !apply_bucket(bucket_last_key, bucket_len, batch)? {
            break;
        }
    }

    Ok(())
}

pub(super) fn apply_event_buckets_infallible<S, F>(
    snapshot_id: u128,
    events: &BTreeMap<ContimeKey, S::Event>,
    start: Bound<ContimeKey>,
    end: Bound<ContimeKey>,
    mut apply_bucket: F,
) where
    S: Snapshot + ApplyEvents + 'static,
    F: FnMut(&ContimeKey, usize, ApplyBatch<'_, S::Event>),
{
    let result = apply_event_buckets::<S, Infallible, _>(snapshot_id, events, start, end, |bucket_last_key, bucket_len, batch| {
        apply_bucket(bucket_last_key, bucket_len, batch);
        Ok(true)
    });
    debug_assert!(result.is_ok());
    if let Err(error) = result {
        unreachable!("infallible bucket apply returned an error: {error}");
    }
}

pub(super) fn get_checkpoint_at<S>(history: &LocalSnapshotHistory<S>, time: i64) -> S
where
    S: Snapshot + ApplyEvents + 'static,
{
    let checkpoint_boundary = last_key_at_time(time);

    let checkpoint_entry = latest_checkpoint_at_or_before(&history.checkpoints, &checkpoint_boundary);

    let (mut snapshot, recompute_start) = match checkpoint_entry {
        Some((key, checkpoint)) => (checkpoint.clone(), Bound::Excluded(key.clone())),
        None => (history.base_snapshot.clone(), Bound::Unbounded),
    };

    let end_key = last_key_at_time(time);

    apply_event_buckets_infallible::<S, _>(
        history.snapshot_id,
        &history.events,
        recompute_start,
        Bound::Included(end_key),
        |_bucket_last_key, _bucket_len, batch| {
            snapshot.apply_events(batch);
            snapshot.set_time(batch.time);
        },
    );

    snapshot.set_time(time);

    snapshot
}
