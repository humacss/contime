use std::collections::{btree_map, vec_deque, BTreeMap, VecDeque};
use std::iter::Peekable;

use ahash::AHashSet;

/// The identity and timestamp required to store an event canonically.
///
/// Both values must remain stable while the event is retained, including when
/// the stored type is a shared handle with interior mutability. History keys and
/// deduplication are recorded at insertion and are not recomputed on reads.
pub trait Event {
    /// The event's ordered time representation.
    ///
    /// `Default` must produce the zero timestamp used by an empty history.
    type Time: Clone + Default + Ord;

    /// Returns the stable identity used for retained-history deduplication.
    fn event_id(&self) -> u128;

    /// Returns the event's canonical ordering time.
    fn time(&self) -> Self::Time;
}

/// The total ordering key for one retained event.
///
/// Time is compared first. Event ID deterministically orders events that share
/// a time.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct EventKey<T> {
    /// Timestamp used as the primary ordering component.
    pub time: T,
    /// Stable event identity, used to order events sharing a timestamp.
    pub event_id: u128,
}

impl<T> EventKey<T> {
    pub(crate) fn from_event<E>(event: &E) -> Self
    where
        E: Event<Time = T>,
    {
        Self { time: event.time(), event_id: event.event_id() }
    }
}

/// Canonical retained events for one externally owned snapshot ID.
///
/// Events arriving after the current canonical tail use the append deque.
/// Events arriving before the tail use the late-event tree. Event IDs remain
/// retained independently so the same ID is a no-op even if its timestamp or
/// payload differs.
pub struct EventHistory<E>
where
    E: Event,
{
    pub(crate) ordered: VecDeque<(EventKey<E::Time>, E)>,
    pub(crate) late: BTreeMap<EventKey<E::Time>, E>,
    pub(crate) retained_ids: AHashSet<u128>,
    pub(crate) dirty_time: E::Time,
    pub(crate) dirty: bool,
    pub(crate) horizon: E::Time,
}

/// The outcome of one event insertion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Insert {
    /// The event was retained.
    Inserted,
    /// The event ID is already retained. Timestamp and payload are ignored.
    Duplicate,
    /// The event predates the active retained-history horizon.
    BeforeHorizon,
}

/// The number of events removed from each internal event store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PruneResult {
    /// Events removed from the ordered append deque.
    pub removed_ordered: usize,
    /// Events removed from the out-of-order event tree.
    pub removed_late: usize,
}

/// A canonical merged view over ordered and late retained events.
pub struct EventHistoryIter<'a, E>
where
    E: Event,
{
    pub(crate) ordered: Peekable<vec_deque::Iter<'a, (EventKey<E::Time>, E)>>,
    pub(crate) late: Peekable<btree_map::Iter<'a, EventKey<E::Time>, E>>,
}

/// A canonical merged view beginning at a history boundary.
pub struct EventHistoryRangeIter<'a, E>
where
    E: Event,
{
    pub(crate) ordered: Peekable<vec_deque::Iter<'a, (EventKey<E::Time>, E)>>,
    pub(crate) late: Peekable<btree_map::Range<'a, EventKey<E::Time>, E>>,
}
