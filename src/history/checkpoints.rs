use std::collections::{BTreeMap, VecDeque};
use std::ops::Bound;

use crate::{ApplyBatch, ApplyDecision, ApplyEvents, ApplyInner, ApplyWrapper, ContimeKey, ContimeTime, Snapshot};

use super::history::LocalSnapshotHistory;

pub(super) fn first_key_at_time<T: ContimeTime>(time: T) -> ContimeKey<T> {
    ContimeKey { time, id: u128::MIN }
}

pub(super) fn last_key_at_time<T: ContimeTime>(time: T) -> ContimeKey<T> {
    ContimeKey { time, id: u128::MAX }
}

pub(super) fn checkpoint_partition_before<T: ContimeTime, S>(
    checkpoints: &VecDeque<(ContimeKey<T>, S)>,
    boundary: &ContimeKey<T>,
) -> usize {
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

pub(super) fn latest_checkpoint_before_index<T: ContimeTime, S>(
    checkpoints: &VecDeque<(ContimeKey<T>, S)>,
    boundary: &ContimeKey<T>,
) -> Option<usize> {
    let index = checkpoint_partition_before(checkpoints, boundary);
    (index > 0).then(|| index - 1)
}

pub(super) fn latest_checkpoint_before<'a, T: ContimeTime, S>(
    checkpoints: &'a VecDeque<(ContimeKey<T>, S)>,
    boundary: &ContimeKey<T>,
) -> Option<(&'a ContimeKey<T>, &'a S)> {
    latest_checkpoint_before_index(checkpoints, boundary).map(|index| {
        let (key, checkpoint) = &checkpoints[index];
        (key, checkpoint)
    })
}

pub(super) fn latest_checkpoint_at_or_before<'a, T: ContimeTime, S>(
    checkpoints: &'a VecDeque<(ContimeKey<T>, S)>,
    boundary: &ContimeKey<T>,
) -> Option<(&'a ContimeKey<T>, &'a S)> {
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

pub(super) fn push_checkpoint<S>(checkpoints: &mut VecDeque<(ContimeKey<S::Time>, S)>, key: ContimeKey<S::Time>, checkpoint: S) -> i64
where
    S: Snapshot,
{
    let bytes_delta = checkpoint.conservative_size() as i64;
    checkpoints.push_back((key, checkpoint));
    bytes_delta
}

pub(super) fn drain_checkpoints_from<S>(checkpoints: &mut VecDeque<(ContimeKey<S::Time>, S)>, start: usize) -> i64
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
    start: Bound<ContimeKey<S::Time>>,
    end: Bound<ContimeKey<S::Time>>,
    stale_start: Option<usize>,
    preserve_previous_tip: bool,
    first_changed_time: S::Time,
    bytes_delta: i64,
}

pub(super) struct AppliedCheckpoint<S>
where
    S: Snapshot,
{
    stale_start: Option<usize>,
    bytes_delta: i64,
    final_key: Option<ContimeKey<S::Time>>,
    final_snapshot: S,
    materialized_checkpoints: Vec<(ContimeKey<S::Time>, S)>,
}

pub(super) fn get_checkpoint_for_apply<S>(
    history: &mut LocalSnapshotHistory<S>,
    earliest_changed_time: S::Time,
    latest_event_key_before_apply: Option<ContimeKey<S::Time>>,
    _single_changed_event_key: Option<ContimeKey<S::Time>>,
    changed_event_count: usize,
) -> CheckpointForApply<S>
where
    S: Snapshot + ApplyEvents + 'static,
{
    let preserve_previous_tip = latest_event_key_before_apply.as_ref().is_none_or(|latest| earliest_changed_time > latest.time);
    let previous_event_count = history.events.len().saturating_sub(changed_event_count);
    let protected_previous_tip =
        latest_event_key_before_apply.as_ref().filter(|_| preserve_previous_tip && is_event_count_cadence(history, previous_event_count));

    get_recomputed_checkpoint_for_apply(history, earliest_changed_time, preserve_previous_tip, protected_previous_tip)
}

pub(super) fn apply_events_to_checkpoint<S, C>(
    history: &LocalSnapshotHistory<S>,
    mut checkpoint: CheckpointForApply<S>,
    context: &mut C,
) -> AppliedCheckpoint<S>
where
    S: Snapshot + ApplyEvents + 'static,
    C: ApplyWrapper<S>,
{
    let mut event_count = 0usize;
    let mut materialized_checkpoints = Vec::new();
    let mut stored_first_changed_checkpoint = false;

    apply_event_buckets::<S, _>(
        history.snapshot_id,
        &history.events,
        checkpoint.start.clone(),
        checkpoint.end.clone(),
        |bucket_last_key, bucket_len, batch| {
            let batch_time = batch.time.clone();
            if context.apply_event_batch_wrapper(&mut checkpoint.snapshot, batch, ApplyInner::default()) == ApplyDecision::EarlyExit {
                return false;
            }
            event_count += bucket_len;

            if !checkpoint.preserve_previous_tip && !stored_first_changed_checkpoint && batch_time >= checkpoint.first_changed_time {
                materialized_checkpoints.push((bucket_last_key.clone(), checkpoint.snapshot.clone()));
                stored_first_changed_checkpoint = true;
            }

            if checkpoint.preserve_previous_tip && checkpoint.end == Bound::Unbounded && history.checkpoint_interval != 0 {
                if event_count % history.checkpoint_interval == 0 {
                    materialized_checkpoints.push((bucket_last_key.clone(), checkpoint.snapshot.clone()));
                }
            }

            true
        },
    );

    AppliedCheckpoint {
        stale_start: checkpoint.stale_start,
        bytes_delta: checkpoint.bytes_delta,
        final_key: history.events.keys().next_back().cloned(),
        final_snapshot: checkpoint.snapshot,
        materialized_checkpoints,
    }
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

fn get_recomputed_checkpoint_for_apply<S>(
    history: &LocalSnapshotHistory<S>,
    time: S::Time,
    preserve_previous_tip: bool,
    protected_previous_tip: Option<&ContimeKey<S::Time>>,
) -> CheckpointForApply<S>
where
    S: Snapshot + ApplyEvents + 'static,
{
    let mut recompute_boundary = first_key_at_time(time.clone());
    if !preserve_previous_tip {
        if let Some((key, _checkpoint)) = latest_checkpoint_before(&history.checkpoints, &recompute_boundary) {
            if !checkpoint_key_is_cadence(history, key) {
                recompute_boundary = key.clone();
            }
        }
    }

    let checkpoint_index = latest_checkpoint_before_index(&history.checkpoints, &recompute_boundary);
    let remove_recompute_checkpoint = preserve_previous_tip
        && checkpoint_index.is_some_and(|index| {
            let key = &history.checkpoints[index].0;
            protected_previous_tip != Some(key)
                && history.checkpoints.back().map(|(back_key, _checkpoint)| back_key == key).unwrap_or(false)
        });
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
        bytes_delta: 0,
    }
}

fn is_event_count_cadence<S>(history: &LocalSnapshotHistory<S>, event_count: usize) -> bool
where
    S: Snapshot,
{
    history.checkpoint_interval != 0 && event_count != 0 && event_count % history.checkpoint_interval == 0
}

fn checkpoint_key_is_cadence<S>(history: &LocalSnapshotHistory<S>, checkpoint_key: &ContimeKey<S::Time>) -> bool
where
    S: Snapshot,
{
    if history.checkpoint_interval == 0 || history.checkpoints.back().map(|(key, _checkpoint)| key) != Some(checkpoint_key) {
        return true;
    }

    is_event_count_cadence(history, history.events.len())
}

pub(super) fn apply_event_buckets<S, F>(
    snapshot_id: u128,
    events: &BTreeMap<ContimeKey<S::Time>, S::Event>,
    start: Bound<ContimeKey<S::Time>>,
    end: Bound<ContimeKey<S::Time>>,
    mut apply_bucket: F,
) where
    S: Snapshot + ApplyEvents + 'static,
    F: FnMut(&ContimeKey<S::Time>, usize, ApplyBatch<'_, S::Event>) -> bool,
{
    let mut iter = events.range((start, end)).peekable();
    while let Some((first_key, first_event)) = iter.next() {
        let bucket_time = first_key.time.clone();
        let mut bucket_last_key = first_key;

        if iter.peek().is_none_or(|(next_key, _next_event)| next_key.time != bucket_time) {
            let bucket = [first_event];
            let batch = ApplyBatch { snapshot_id, time: bucket_time, events: &bucket };
            if !apply_bucket(bucket_last_key, 1, batch) {
                break;
            }
            continue;
        }

        let (second_key, second_event) = iter.next().expect("same-time bucket second event must exist");
        bucket_last_key = second_key;

        if iter.peek().is_none_or(|(next_key, _next_event)| next_key.time != bucket_time) {
            let bucket = [first_event, second_event];
            let batch = ApplyBatch { snapshot_id, time: bucket_time, events: &bucket };
            if !apply_bucket(bucket_last_key, 2, batch) {
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
        if !apply_bucket(bucket_last_key, bucket_len, batch) {
            break;
        }
    }
}

pub(super) fn get_checkpoint_at<S>(history: &LocalSnapshotHistory<S>, time: S::Time) -> S
where
    S: Snapshot + ApplyEvents + 'static,
{
    let mut context = ();
    get_checkpoint_at_with_context(history, time, &mut context)
}

pub(super) fn get_checkpoint_at_with_context<S, C>(history: &LocalSnapshotHistory<S>, time: S::Time, context: &mut C) -> S
where
    S: Snapshot + ApplyEvents + 'static,
    C: ApplyWrapper<S>,
{
    let checkpoint_boundary = last_key_at_time(time.clone());

    let checkpoint_entry = latest_checkpoint_at_or_before(&history.checkpoints, &checkpoint_boundary);

    let (mut snapshot, recompute_start) = match checkpoint_entry {
        Some((key, checkpoint)) => (checkpoint.clone(), Bound::Excluded(key.clone())),
        None => (history.base_snapshot.clone(), Bound::Unbounded),
    };

    let end_key = last_key_at_time(time.clone());

    apply_event_buckets::<S, _>(
        history.snapshot_id,
        &history.events,
        recompute_start,
        Bound::Included(end_key),
        |_bucket_last_key, _bucket_len, batch| {
            let batch_time = batch.time.clone();
            context.apply_event_batch_wrapper(&mut snapshot, batch, ApplyInner::default());
            snapshot.set_time(batch_time);
            true
        },
    );

    snapshot.set_time(time);

    snapshot
}

pub(super) fn get_checkpoint_before_with_context<S, C>(history: &LocalSnapshotHistory<S>, time: S::Time, context: &mut C) -> S
where
    S: Snapshot + ApplyEvents + 'static,
    C: ApplyWrapper<S>,
{
    let boundary = first_key_at_time(time);
    let checkpoint_entry = latest_checkpoint_before(&history.checkpoints, &boundary);

    let (mut snapshot, recompute_start) = match checkpoint_entry {
        Some((key, checkpoint)) => (checkpoint.clone(), Bound::Excluded(key.clone())),
        None => (history.base_snapshot.clone(), Bound::Unbounded),
    };

    apply_event_buckets::<S, _>(
        history.snapshot_id,
        &history.events,
        recompute_start,
        Bound::Excluded(boundary),
        |_bucket_last_key, _bucket_len, batch| {
            let batch_time = batch.time.clone();
            context.apply_event_batch_wrapper(&mut snapshot, batch, ApplyInner::default());
            snapshot.set_time(batch_time);
            true
        },
    );

    snapshot
}
