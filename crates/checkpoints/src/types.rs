use std::collections::VecDeque;

/// Checkpoint retention policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckpointConfig {
    /// Number of applied events between retained cadence checkpoints.
    ///
    /// Zero disables cadence checkpoints while retaining the current tip.
    pub interval: u64,
}

/// The canonical position of an event or checkpoint.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CheckpointKey<T> {
    pub time: T,
    pub event_id: u128,
}

/// One borrowed canonical event supplied by an event store.
#[derive(Clone, Copy, Debug)]
pub struct EventRef<'a, T, E> {
    pub time: &'a T,
    pub event_id: u128,
    pub event: &'a E,
}

/// Canonical events and replay acknowledgement required by apply-time replay.
pub trait Events {
    type Time: Clone + Default + Ord;
    type Event;
    type Iter<'a>: Iterator<Item = EventRef<'a, Self::Time, Self::Event>>
    where
        Self: 'a,
        Self::Time: 'a,
        Self::Event: 'a;

    /// Returns the earliest timestamp changed since the previous replay.
    fn dirty_time(&self) -> &Self::Time;

    /// Iterates canonically after `boundary`, or from the beginning when it is
    /// absent.
    fn iter_after(&self, boundary: Option<&CheckpointKey<Self::Time>>) -> Self::Iter<'_>;

    /// Acknowledges that the canonical events exposed by the completed replay
    /// have been reflected in checkpoint state.
    ///
    /// Replay calls this exactly once after successful completion, including
    /// when no events required application. A replay that panics is not
    /// acknowledged.
    fn acknowledge_replay(&mut self);
}

/// Consumer-owned state retained in checkpoints.
pub trait Snapshot: Clone {
    type Time: Clone + Default + Ord;

    /// Updates the materialized state's logical time after applying a bucket.
    fn set_time(&mut self, time: Self::Time);
}

/// One complete same-time event bucket.
#[derive(Clone, Debug)]
pub struct EventBatch<'a, T, E> {
    pub snapshot_id: u128,
    pub time: T,
    pub events: &'a [&'a E],
}

/// One effective event batch selected from a canonical timestamp bucket.
#[derive(Clone, Debug)]
pub struct ApplyBatch<'a, T, E> {
    pub snapshot_id: u128,
    pub time: T,
    /// Cumulative raw history event count represented through the canonical
    /// bucket from which this effective batch was selected.
    pub history_event_count: u64,
    pub events: &'a [&'a E],
}

/// Consumer-provided snapshot materialization and event application behavior.
pub trait ApplyEvents<E>: Snapshot {
    /// Creates clean state with the identity selected by the first event.
    fn create(snapshot_id: u128, first_event: &E) -> Self;

    /// Applies one effective event batch.
    fn apply_events(&mut self, batch: ApplyBatch<'_, Self::Time, E>);
}

/// The only mutable snapshot access exposed to apply wrappers.
pub struct ApplyInner<'a, S>
where
    S: Snapshot,
{
    pub(crate) snapshot: &'a mut S,
    pub(crate) history_event_count: u64,
    pub(crate) apply_count: usize,
}

/// Infallible extension seam around same-timestamp snapshot application.
///
/// Implementations must call `ApplyInner::apply_event_batch` at least once.
/// They may filter or partition the canonical batch and may use an empty
/// effective batch to suppress every event.
pub trait ApplyWrapper<S, E>
where
    S: ApplyEvents<E>,
{
    fn apply_event_batch(&mut self, batch: EventBatch<'_, S::Time, E>, apply_inner: &mut ApplyInner<'_, S>) {
        apply_inner.apply_event_batch(batch);
    }
}

impl<S, E> ApplyWrapper<S, E> for () where S: ApplyEvents<E> {}

/// One retained materialized checkpoint.
#[derive(Clone, Debug)]
pub struct Checkpoint<S>
where
    S: Snapshot,
{
    pub key: CheckpointKey<S::Time>,
    pub snapshot: S,
    pub history_event_count: u64,
}

/// Materialized checkpoints for one externally scheduled snapshot ID.
#[derive(Clone)]
pub struct CheckpointStore<S>
where
    S: Snapshot,
{
    pub(crate) snapshot_id: u128,
    pub(crate) interval: u64,
    pub(crate) checkpoints: VecDeque<Checkpoint<S>>,
}

/// Checkpoint-owned state used while one replay walks canonical events.
pub(crate) struct ReplaySession<'a, S>
where
    S: Snapshot,
{
    pub(crate) store: &'a mut CheckpointStore<S>,
    pub(crate) working_snapshot: Option<S>,
    pub(crate) start_key: Option<CheckpointKey<S::Time>>,
    pub(crate) history_event_count: u64,
    pub(crate) applied_events: u64,
    pub(crate) events_since_checkpoint: u64,
    pub(crate) next_checkpoint_index: usize,
}

/// The effect of one apply-time replay on retained checkpoint state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApplyResult {
    /// Number of canonical events applied during this replay.
    pub applied_events: u64,
    /// Total checkpoints retained after this replay.
    pub retained_checkpoints: usize,
}
