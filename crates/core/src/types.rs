use std::marker::PhantomData;

#[cfg(test)]
pub(crate) mod testing {
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
    pub struct Time(pub i64);

    impl contime_checkpoints::Timestamp for Time {
        fn previous(&self) -> Option<Self> {
            self.0.checked_sub(1).map(Self)
        }
    }
    impl contime_worker::AdvanceTime for Time {
        fn saturating_sub(&self, other: &Self) -> Self {
            Self(self.0.saturating_sub(other.0))
        }
    }
}
use std::sync::Arc;
/// The event information required by the complete apply pipeline.
pub trait Input:
    contime_checkpoints::Event<Time: contime_worker::AdvanceTime + contime_checkpoints::Timestamp + 'static> + 'static
{
    /// Returns a stable identity for duplicate detection within each history.
    ///
    /// Retained events are ordered by timestamp and this ID. Reusing an ID for
    /// different payloads in one history can cause the later input to be ignored.
    fn event_id(&self) -> u128;
    /// Emits the snapshot histories affected by this input.
    ///
    /// Routing must be deterministic for the same input. Every emitted ID creates
    /// a route, including duplicates; emitting no IDs drops the input from routing.
    /// Histories share immutable event ownership and deduplicate retained event IDs.
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128));
}

/// An immutable event shared by its routed snapshot histories.
pub struct SharedEvent<I>
where
    I: Input,
{
    pub(crate) inner: Arc<I>,
}

/// A core-owned reason returned at the public apply boundary.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RejectionReason {
    /// An external input precedes the requested retention boundary, or a target
    /// history can no longer insert an input at its timestamp.
    BeforeHistoryHorizon,
    /// Internally generated work precedes the coordinator's committed safe boundary.
    BeforeSafeTime,
    /// Internally generated work precedes the timestamp of its causal source.
    BeforeSourceTime,
}

/// One rejected insertion, identified independently of any response transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InsertionRejection {
    /// Identity supplied by the rejected input.
    pub event_id: u128,
    /// Why this history could not accept the input.
    pub reason: RejectionReason,
}

pub(crate) struct History<I>
where
    I: Input,
{
    pub(crate) events: contime_events::EventHistory<SharedEvent<I>>,
}

pub(crate) enum HistoryIter<'a, I>
where
    I: Input,
{
    All(contime_events::EventHistoryIter<'a, SharedEvent<I>>),
    Range(contime_events::EventHistoryRangeIter<'a, SharedEvent<I>>),
}

/// Settings for the shared synchronous worker storage adapter.
pub struct CheckpointStorageConfig {
    /// Checkpoint spacing used for each history.
    pub checkpoints: crate::checkpoints::CheckpointConfig,
}

/// Synchronous storage adapter shared by execution modes.
/// Implements the existing worker storage contract without channels or thread bounds.
pub struct CheckpointStorage<I, S, W>
where
    I: Input,
    S: contime_checkpoints::Snapshot<Time = I::Time>,
{
    pub(crate) snapshot_id: u128,
    pub(crate) horizon: I::Time,
    pub(crate) interval: u64,
    pub(crate) store: Option<contime_checkpoints::SnapshotStore<S, History<I>>>,
    pub(crate) wrapper: PhantomData<fn() -> W>,
}
