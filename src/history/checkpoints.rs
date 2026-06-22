use std::collections::{BTreeMap, VecDeque};
use std::ops::Bound;

use crate::{ApplyBatch, ApplyError, ApplyEvents, ApplyInner, ApplyWrapper, ContimeKey, Event, Snapshot};

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

pub(super) fn update_checkpoints_after_event_batch<S, C>(
    history: &mut LocalSnapshotHistory<S>,
    earliest_changed_time: i64,
    latest_changed_time: i64,
    latest_event_key_before_apply: Option<ContimeKey>,
    single_changed_event_key: Option<ContimeKey>,
    context: &mut C,
) -> Result<i64, ApplyError>
where
    S: Snapshot + ApplyEvents + 'static,
    C: ApplyWrapper<S>,
{
    let (extra_events, earliest_apply_time) =
        collect_wrapper_batches_for_changed_event_times(history, earliest_changed_time, latest_changed_time, context)?;
    let preserve_previous_tip = latest_event_key_before_apply.as_ref().is_none_or(|latest| earliest_apply_time > latest.time);

    if preserve_previous_tip {
        if let Some(single_changed_event_key) = single_changed_event_key {
            return apply_latest_event_without_recompute(
                history,
                latest_event_key_before_apply,
                single_changed_event_key,
                earliest_apply_time,
                &extra_events,
                context,
            );
        }
    }

    recompute_checkpoints_from(history, earliest_apply_time, preserve_previous_tip, &extra_events, context)
}

fn collect_wrapper_batches_for_changed_event_times<S, C>(
    history: &LocalSnapshotHistory<S>,
    earliest_changed_time: i64,
    latest_changed_time: i64,
    context: &mut C,
) -> Result<(BTreeMap<ContimeKey, S::Event>, i64), ApplyError>
where
    S: Snapshot + ApplyEvents + 'static,
    C: ApplyWrapper<S>,
{
    let mut extra_events = BTreeMap::new();
    let mut earliest_apply_time = earliest_changed_time;
    let start = Bound::Included(first_key_at_time(earliest_changed_time));
    let end = Bound::Included(last_key_at_time(latest_changed_time));

    apply_event_buckets::<S, _>(history.snapshot_id, &history.events, start, end, |_bucket_last_key, _bucket_len, batch| {
        for extra_batch in context.extra_batches(batch).map_err(Into::into)? {
            let Some(first_event) = extra_batch.first() else {
                continue;
            };
            let batch_time = first_event.time();
            if batch_time > batch.time {
                return Err(ApplyError::new(format!("extra batch at {} cannot be after the input batch at {}", batch_time, batch.time)));
            }
            for event in extra_batch {
                if event.time() != batch_time {
                    return Err(ApplyError::new("extra batch contains events from multiple timestamps"));
                }
                earliest_apply_time = earliest_apply_time.min(event.time());
                extra_events.insert(ContimeKey::from_event(&event), event);
            }
        }
        Ok(())
    })?;

    Ok((extra_events, earliest_apply_time))
}

fn apply_latest_event_without_recompute<S, C>(
    history: &mut LocalSnapshotHistory<S>,
    latest_event_key_before_apply: Option<ContimeKey>,
    single_changed_event_key: ContimeKey,
    earliest_apply_time: i64,
    extra_events: &BTreeMap<ContimeKey, S::Event>,
    context: &mut C,
) -> Result<i64, ApplyError>
where
    S: Snapshot + ApplyEvents + 'static,
    C: ApplyWrapper<S>,
{
    let previous_event_count = history.events.len().saturating_sub(1);
    let previous_tip_is_cadence =
        latest_event_key_before_apply.as_ref().is_some_and(|_key| is_event_count_cadence(history, previous_event_count));

    let mut bytes_delta = 0;
    let mut snapshot = if let Some(previous_key) = latest_event_key_before_apply.as_ref().filter(|_key| !previous_tip_is_cadence) {
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

    apply_event_buckets_with_extra::<S, _>(
        history.snapshot_id,
        &history.events,
        Bound::Included(first_key_at_time(earliest_apply_time)),
        Bound::Included(single_changed_event_key.clone()),
        extra_events,
        |_bucket_last_key, _bucket_len, batch| {
            context.apply_event_batch_wrapper(&mut snapshot, batch, ApplyInner::default()).map_err(Into::into)?;
            Ok(())
        },
    )?;

    bytes_delta += push_checkpoint(&mut history.checkpoints, single_changed_event_key, snapshot);
    Ok(bytes_delta)
}

fn recompute_checkpoints_from<S, C>(
    history: &mut LocalSnapshotHistory<S>,
    time: i64,
    preserve_previous_tip: bool,
    extra_events: &BTreeMap<ContimeKey, S::Event>,
    context: &mut C,
) -> Result<i64, ApplyError>
where
    S: Snapshot + ApplyEvents + 'static,
    C: ApplyWrapper<S>,
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
    let (mut snapshot, recompute_start) = match checkpoint_index {
        Some(index) => {
            let (key, checkpoint) = &history.checkpoints[index];
            (checkpoint.clone(), Bound::Excluded(key.clone()))
        }
        None => (history.base_snapshot.clone(), Bound::Unbounded),
    };

    let mut bytes_delta = 0;
    let stale_start = if remove_recompute_checkpoint {
        checkpoint_index.expect("remove checkpoint index must exist")
    } else {
        checkpoint_partition_before(&history.checkpoints, &recompute_boundary)
    };
    bytes_delta += drain_checkpoints_from(&mut history.checkpoints, stale_start);

    let mut event_count = 0usize;
    let events = &history.events;
    let checkpoints = &mut history.checkpoints;
    let snapshot_id = history.snapshot_id;
    let checkpoint_interval = history.checkpoint_interval;
    let mut stored_first_changed_checkpoint = false;

    apply_event_buckets_with_extra::<S, _>(
        snapshot_id,
        events,
        recompute_start,
        Bound::Unbounded,
        extra_events,
        |bucket_last_key, bucket_len, batch| {
            context.apply_event_batch_wrapper(&mut snapshot, batch, ApplyInner::default()).map_err(Into::into)?;
            event_count += bucket_len;

            if !preserve_previous_tip && !stored_first_changed_checkpoint && batch.time >= time {
                bytes_delta += push_checkpoint(checkpoints, bucket_last_key.clone(), snapshot.clone());
                stored_first_changed_checkpoint = true;
            }

            if preserve_previous_tip && checkpoint_interval != 0 && event_count % checkpoint_interval == 0 {
                bytes_delta += push_checkpoint(checkpoints, bucket_last_key.clone(), snapshot.clone());
            }
            Ok(())
        },
    )?;

    if let Some(latest_key) = events.keys().next_back().cloned() {
        if checkpoints.back().map(|(key, _checkpoint)| key) != Some(&latest_key) {
            bytes_delta += push_checkpoint(checkpoints, latest_key, snapshot);
        }
    }

    Ok(bytes_delta)
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

pub(super) fn apply_event_buckets<S, F>(
    snapshot_id: u128,
    events: &BTreeMap<ContimeKey, S::Event>,
    start: Bound<ContimeKey>,
    end: Bound<ContimeKey>,
    mut apply_bucket: F,
) -> Result<(), ApplyError>
where
    S: Snapshot + ApplyEvents + 'static,
    F: FnMut(&ContimeKey, usize, ApplyBatch<'_, S::Event>) -> Result<(), ApplyError>,
{
    let mut iter = events.range((start, end)).peekable();
    while let Some((first_key, first_event)) = iter.next() {
        let bucket_time = first_key.time;
        let mut bucket_last_key = first_key;

        if iter.peek().is_none_or(|(next_key, _next_event)| next_key.time != bucket_time) {
            let bucket = [first_event];
            let batch = ApplyBatch { snapshot_id, time: bucket_time, events: &bucket };
            apply_bucket(bucket_last_key, 1, batch)?;
            continue;
        }

        let (second_key, second_event) = iter.next().expect("same-time bucket second event must exist");
        bucket_last_key = second_key;

        if iter.peek().is_none_or(|(next_key, _next_event)| next_key.time != bucket_time) {
            let bucket = [first_event, second_event];
            let batch = ApplyBatch { snapshot_id, time: bucket_time, events: &bucket };
            apply_bucket(bucket_last_key, 2, batch)?;
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
        apply_bucket(bucket_last_key, bucket_len, batch)?;
    }

    Ok(())
}

pub(super) fn apply_event_buckets_with_extra<S, F>(
    snapshot_id: u128,
    events: &BTreeMap<ContimeKey, S::Event>,
    start: Bound<ContimeKey>,
    end: Bound<ContimeKey>,
    extra_events: &BTreeMap<ContimeKey, S::Event>,
    mut apply_bucket: F,
) -> Result<(), ApplyError>
where
    S: Snapshot + ApplyEvents + 'static,
    F: FnMut(&ContimeKey, usize, ApplyBatch<'_, S::Event>) -> Result<(), ApplyError>,
{
    let mut real_iter = events.range((start, end)).peekable();
    let mut extra_iter = extra_events.iter().peekable();

    loop {
        let next_real_time = real_iter.peek().map(|(key, _event)| key.time);
        let next_extra_time = extra_iter.peek().map(|(key, _event)| key.time);
        let Some(bucket_time) = next_real_time.into_iter().chain(next_extra_time).min() else {
            break;
        };

        let mut bucket = Vec::new();
        let mut bucket_last_key = None;

        while let Some((key, event)) = real_iter.peek() {
            if key.time != bucket_time {
                break;
            }
            bucket_last_key = Some((*key).clone());
            bucket.push((*key, *event));
            real_iter.next();
        }

        while let Some((key, event)) = extra_iter.peek() {
            if key.time != bucket_time {
                break;
            }
            bucket_last_key = Some((*key).clone());
            bucket.push((*key, *event));
            extra_iter.next();
        }

        bucket.sort_by(|(left_key, _left_event), (right_key, _right_event)| left_key.cmp(right_key));
        let bucket_last_key = bucket_last_key.expect("bucket must contain at least one event");
        let bucket_events: Vec<_> = bucket.into_iter().map(|(_key, event)| event).collect();
        let batch = ApplyBatch { snapshot_id, time: bucket_time, events: &bucket_events };
        apply_bucket(&bucket_last_key, bucket_events.len(), batch)?;
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
    let result = apply_event_buckets::<S, _>(snapshot_id, events, start, end, |bucket_last_key, bucket_len, batch| {
        apply_bucket(bucket_last_key, bucket_len, batch);
        Ok(())
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
