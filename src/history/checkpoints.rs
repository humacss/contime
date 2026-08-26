use std::collections::{BTreeMap, VecDeque};
use std::ops::Bound;

use crate::{ApplyInner, ApplyWrapper, ContimeKey, ContimeTime, InputBatch, InputLanes, Snapshot, SnapshotLanes};

use super::storage::LocalSnapshotHistory;

pub(super) fn first_key_at_time<T: ContimeTime>(time: T) -> ContimeKey<T> {
    ContimeKey { time, id: u128::MIN }
}

pub(super) fn last_key_at_time<T: ContimeTime>(time: T) -> ContimeKey<T> {
    ContimeKey { time, id: u128::MAX }
}

pub(super) fn checkpoint_partition_before<T: ContimeTime, S>(
    checkpoints: &VecDeque<(ContimeKey<T>, S, u64)>,
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
    checkpoints: &VecDeque<(ContimeKey<T>, S, u64)>,
    boundary: &ContimeKey<T>,
) -> Option<usize> {
    let index = checkpoint_partition_before(checkpoints, boundary);
    (index > 0).then(|| index - 1)
}

pub(super) fn latest_checkpoint_before<'a, T: ContimeTime, S>(
    checkpoints: &'a VecDeque<(ContimeKey<T>, S, u64)>,
    boundary: &ContimeKey<T>,
) -> Option<(&'a ContimeKey<T>, &'a S, u64)> {
    latest_checkpoint_before_index(checkpoints, boundary).map(|index| {
        let (key, checkpoint, history_input_count) = &checkpoints[index];
        (key, checkpoint, *history_input_count)
    })
}

pub(super) fn latest_checkpoint_at_or_before<'a, T: ContimeTime, S>(
    checkpoints: &'a VecDeque<(ContimeKey<T>, S, u64)>,
    boundary: &ContimeKey<T>,
) -> Option<(&'a ContimeKey<T>, &'a S, u64)> {
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
        let (key, checkpoint, history_input_count) = &checkpoints[index - 1];
        (key, checkpoint, *history_input_count)
    })
}

pub(super) fn push_checkpoint<S>(
    checkpoints: &mut VecDeque<(ContimeKey<S::Time>, S, u64)>,
    key: ContimeKey<S::Time>,
    checkpoint: S,
    history_input_count: u64,
) -> i64
where
    S: Snapshot,
{
    let bytes_delta = checkpoint.conservative_size() as i64 + size_of::<u64>() as i64;
    checkpoints.push_back((key, checkpoint, history_input_count));
    bytes_delta
}

pub(super) fn drain_checkpoints_from<S>(checkpoints: &mut VecDeque<(ContimeKey<S::Time>, S, u64)>, start: usize) -> i64
where
    S: Snapshot,
{
    let mut bytes_delta = 0;
    for (_key, removed, _history_input_count) in checkpoints.drain(start..) {
        bytes_delta -= removed.conservative_size() as i64 + size_of::<u64>() as i64;
    }
    bytes_delta
}

pub(super) struct CheckpointForApply<S>
where
    S: Snapshot,
{
    snapshot: S,
    history_input_count: u64,
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
    final_history_input_count: u64,
    materialized_checkpoints: Vec<(ContimeKey<S::Time>, S, u64)>,
}

pub(super) fn get_checkpoint_for_apply<S>(
    history: &mut LocalSnapshotHistory<S>,
    earliest_changed_time: S::Time,
    latest_event_key_before_apply: Option<ContimeKey<S::Time>>,
    _single_changed_event_key: Option<ContimeKey<S::Time>>,
    changed_event_count: usize,
) -> Option<CheckpointForApply<S>>
where
    S: SnapshotLanes + 'static,
    S::Input: InputLanes<S>,
{
    let preserve_previous_tip = latest_event_key_before_apply.as_ref().is_none_or(|latest| earliest_changed_time > latest.time);
    let previous_event_count = history.inputs.len().saturating_sub(changed_event_count);
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
    S: SnapshotLanes + 'static,
    S::Input: InputLanes<S>,
    C: ApplyWrapper<S>,
{
    let mut event_count = 0usize;
    let mut materialized_checkpoints = Vec::new();
    let mut stored_first_changed_checkpoint = false;

    apply_input_buckets::<S, _>(
        history.snapshot_id,
        &history.inputs,
        checkpoint.start.clone(),
        checkpoint.end.clone(),
        |bucket_last_key, bucket_event_count, bucket_input_count, batch| {
            let batch_time = batch.time.clone();
            checkpoint.history_input_count = checkpoint
                .history_input_count
                .checked_add(u64::try_from(bucket_input_count).expect("history input bucket length exceeded u64"))
                .expect("history input count overflow");
            let mut apply_inner = ApplyInner::new(&mut checkpoint.snapshot, checkpoint.history_input_count);
            context.reconcile_input_batch_wrapper(batch, &mut apply_inner);
            assert!(apply_inner.has_applied(), "an apply wrapper must call the inner apply at least once per input batch");
            event_count += bucket_event_count;

            if !checkpoint.preserve_previous_tip && !stored_first_changed_checkpoint && batch_time >= checkpoint.first_changed_time {
                materialized_checkpoints.push((bucket_last_key.clone(), checkpoint.snapshot.clone(), checkpoint.history_input_count));
                stored_first_changed_checkpoint = true;
            }

            if checkpoint.preserve_previous_tip
                && checkpoint.end == Bound::Unbounded
                && history.checkpoint_interval != 0
                && event_count.is_multiple_of(history.checkpoint_interval)
            {
                materialized_checkpoints.push((bucket_last_key.clone(), checkpoint.snapshot.clone(), checkpoint.history_input_count));
            }
        },
    );

    AppliedCheckpoint {
        stale_start: checkpoint.stale_start,
        bytes_delta: checkpoint.bytes_delta,
        final_key: history.latest_input_key(),
        final_snapshot: checkpoint.snapshot,
        final_history_input_count: checkpoint.history_input_count,
        materialized_checkpoints,
    }
}

pub(super) fn commit_applied_checkpoint<S>(history: &mut LocalSnapshotHistory<S>, applied_checkpoint: AppliedCheckpoint<S>) -> i64
where
    S: SnapshotLanes + 'static,
{
    let mut bytes_delta = applied_checkpoint.bytes_delta;

    if let Some(stale_start) = applied_checkpoint.stale_start {
        bytes_delta += drain_checkpoints_from(&mut history.checkpoints, stale_start);
    }

    for (key, checkpoint, history_input_count) in applied_checkpoint.materialized_checkpoints {
        bytes_delta += push_checkpoint(&mut history.checkpoints, key, checkpoint, history_input_count);
    }

    if let Some(latest_key) = applied_checkpoint.final_key {
        if history.checkpoints.back().map(|(key, _checkpoint, _history_input_count)| key) != Some(&latest_key) {
            bytes_delta += push_checkpoint(
                &mut history.checkpoints,
                latest_key,
                applied_checkpoint.final_snapshot,
                applied_checkpoint.final_history_input_count,
            );
        }
    }

    bytes_delta
}

fn get_recomputed_checkpoint_for_apply<S>(
    history: &LocalSnapshotHistory<S>,
    time: S::Time,
    preserve_previous_tip: bool,
    protected_previous_tip: Option<&ContimeKey<S::Time>>,
) -> Option<CheckpointForApply<S>>
where
    S: SnapshotLanes + 'static,
    S::Input: InputLanes<S>,
{
    let mut recompute_boundary = first_key_at_time(time.clone());
    if !preserve_previous_tip {
        if let Some((key, _checkpoint, _history_input_count)) = latest_checkpoint_before(&history.checkpoints, &recompute_boundary) {
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
                && history.replay_anchor_key.as_ref() != Some(key)
                && history.checkpoints.back().map(|(back_key, _checkpoint, _history_input_count)| back_key == key).unwrap_or(false)
        });
    let (snapshot, history_input_count, start) = match checkpoint_index {
        Some(index) => {
            let (key, checkpoint, history_input_count) = &history.checkpoints[index];
            (checkpoint.clone(), *history_input_count, Bound::Excluded(key.clone()))
        }
        None => (materialize_snapshot(history, Bound::Unbounded)?, 0, Bound::Unbounded),
    };

    let stale_start = if remove_recompute_checkpoint {
        checkpoint_index
    } else {
        Some(checkpoint_partition_before(&history.checkpoints, &recompute_boundary))
    };

    Some(CheckpointForApply {
        snapshot,
        history_input_count,
        start,
        end: Bound::Unbounded,
        stale_start,
        preserve_previous_tip,
        first_changed_time: time,
        bytes_delta: 0,
    })
}

fn materialize_snapshot<S>(history: &LocalSnapshotHistory<S>, end: Bound<ContimeKey<S::Time>>) -> Option<S>
where
    S: SnapshotLanes + 'static,
    S::Input: InputLanes<S>,
{
    history.inputs.range((Bound::Unbounded, end)).find_map(|(_key, input)| S::materialize(history.snapshot_id, input))
}

fn is_event_count_cadence<S>(history: &LocalSnapshotHistory<S>, event_count: usize) -> bool
where
    S: Snapshot,
{
    history.checkpoint_interval != 0 && event_count != 0 && event_count.is_multiple_of(history.checkpoint_interval)
}

fn checkpoint_key_is_cadence<S>(history: &LocalSnapshotHistory<S>, checkpoint_key: &ContimeKey<S::Time>) -> bool
where
    S: Snapshot,
{
    if history.checkpoint_interval == 0
        || history.checkpoints.back().map(|(key, _checkpoint, _history_input_count)| key) != Some(checkpoint_key)
    {
        return true;
    }

    is_event_count_cadence(history, history.inputs.len())
}

pub(super) fn apply_input_buckets<S, F>(
    snapshot_id: u128,
    inputs: &BTreeMap<ContimeKey<S::Time>, S::Input>,
    start: Bound<ContimeKey<S::Time>>,
    end: Bound<ContimeKey<S::Time>>,
    mut apply_bucket: F,
) where
    S: SnapshotLanes + 'static,
    S::Input: InputLanes<S>,
    F: FnMut(&ContimeKey<S::Time>, usize, usize, InputBatch<'_, S::Input>),
{
    let mut input_iter = inputs.range((start, end)).peekable();

    while let Some((input_key, _input)) = input_iter.peek() {
        let bucket_time = input_key.time.clone();
        let mut bucket_last_key = ContimeKey { time: bucket_time.clone(), id: u128::MIN };
        let mut input_bucket = Vec::new();

        while input_iter.peek().is_some_and(|(key, _)| key.time == bucket_time) {
            let (key, input) = input_iter.next().expect("peeked input must exist");
            bucket_last_key = bucket_last_key.max(key.clone());
            input_bucket.push(input);
        }

        let event_count = input_bucket.iter().filter(|input| input.is_event()).count();
        let input_count = input_bucket.len();
        apply_bucket(&bucket_last_key, event_count, input_count, InputBatch { snapshot_id, time: bucket_time, inputs: &input_bucket });
    }
}

pub(super) fn get_checkpoint_at<S>(history: &LocalSnapshotHistory<S>, time: S::Time) -> Option<S>
where
    S: SnapshotLanes + 'static,
    S::Input: InputLanes<S>,
    (): ApplyWrapper<S>,
{
    let mut context = ();
    get_checkpoint_at_with_context(history, time, &mut context)
}

pub(super) fn get_checkpoint_at_with_context<S, C>(history: &LocalSnapshotHistory<S>, time: S::Time, context: &mut C) -> Option<S>
where
    S: SnapshotLanes + 'static,
    S::Input: InputLanes<S>,
    C: ApplyWrapper<S>,
{
    let checkpoint_boundary = last_key_at_time(time.clone());

    let checkpoint_entry = latest_checkpoint_at_or_before(&history.checkpoints, &checkpoint_boundary);

    let (mut snapshot, mut history_input_count, recompute_start) = match checkpoint_entry {
        Some((key, checkpoint, history_input_count)) => (checkpoint.clone(), history_input_count, Bound::Excluded(key.clone())),
        None => (materialize_snapshot(history, Bound::Unbounded)?, 0, Bound::Unbounded),
    };

    let end_key = last_key_at_time(time.clone());

    apply_input_buckets::<S, _>(
        history.snapshot_id,
        &history.inputs,
        recompute_start,
        Bound::Included(end_key),
        |_bucket_last_key, _bucket_event_count, bucket_input_count, batch| {
            let batch_time = batch.time.clone();
            history_input_count = history_input_count
                .checked_add(u64::try_from(bucket_input_count).expect("history input bucket length exceeded u64"))
                .expect("history input count overflow");
            let mut apply_inner = ApplyInner::new(&mut snapshot, history_input_count);
            context.apply_input_batch_wrapper(batch, &mut apply_inner);
            assert!(apply_inner.has_applied(), "an apply wrapper must call the inner apply at least once per input batch");
            snapshot.set_time(batch_time);
        },
    );

    snapshot.set_time(time);

    Some(snapshot)
}

pub(super) fn get_checkpoint_before_with_context<S, C>(
    history: &LocalSnapshotHistory<S>,
    time: S::Time,
    context: &mut C,
) -> Option<(ContimeKey<S::Time>, S, u64)>
where
    S: SnapshotLanes + 'static,
    S::Input: InputLanes<S>,
    C: ApplyWrapper<S>,
{
    let boundary = first_key_at_time(time);
    let checkpoint_entry = latest_checkpoint_before(&history.checkpoints, &boundary);

    let (mut snapshot, mut history_input_count, recompute_start) = match checkpoint_entry {
        Some((key, checkpoint, history_input_count)) => (checkpoint.clone(), history_input_count, Bound::Excluded(key.clone())),
        None => (materialize_snapshot(history, Bound::Excluded(boundary.clone()))?, 0, Bound::Unbounded),
    };

    let final_key = history
        .inputs
        .range((Bound::Unbounded, Bound::Excluded(boundary.clone())))
        .next_back()
        .map(|(key, _input)| key.clone())
        .or_else(|| checkpoint_entry.map(|(key, _checkpoint, _history_input_count)| key.clone()))?;

    apply_input_buckets::<S, _>(
        history.snapshot_id,
        &history.inputs,
        recompute_start,
        Bound::Excluded(boundary),
        |_bucket_last_key, _bucket_event_count, bucket_input_count, batch| {
            let batch_time = batch.time.clone();
            history_input_count = history_input_count
                .checked_add(u64::try_from(bucket_input_count).expect("history input bucket length exceeded u64"))
                .expect("history input count overflow");
            let mut apply_inner = ApplyInner::new(&mut snapshot, history_input_count);
            context.apply_input_batch_wrapper(batch, &mut apply_inner);
            assert!(apply_inner.has_applied(), "an apply wrapper must call the inner apply at least once per input batch");
            snapshot.set_time(batch_time);
        },
    );

    Some((final_key, snapshot, history_input_count))
}
