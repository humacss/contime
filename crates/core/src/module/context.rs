//! Repeatable typed views over one effective timestamp bucket.
use super::{EventId, EventRef, EventSetBuilder, EventSource, SnapshotId};
use crate::contime_core::checkpoints;
use std::marker::PhantomData;

/// Application-owned event registration, initialization and timestamp-batch logic.
///
/// Handle event types in an explicit phase order inside `apply_batch`. Each typed
/// view preserves canonical ordering within that type and can be read repeatedly.
/// ConTime never groups distinct effective timestamps into the same batch.
pub trait SnapshotApply: checkpoints::Snapshot + SnapshotId + 'static {
    /// Registers accepted event types once when the module is initialized.
    fn define(events: &mut EventSetBuilder<Self>);
    /// Initializes a targeted identity from its first event, before application.
    fn create(snapshot_id: u128, event: EventRef<'_>) -> Self;
    /// Applies one complete effective timestamp batch in application-defined phases.
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>);
}
/// Metadata and non-consuming event views for one snapshot's timestamp batch.
pub trait ApplyContext<S: SnapshotApply> {
    /// Destination history identity.
    fn snapshot_id(&self) -> u128;
    /// Effective timestamp shared by this batch.
    fn time(&self) -> &S::Time;
    /// Raw canonical history event count preceding this timestamp batch.
    fn history_event_count(&self) -> u64;
    /// Borrows events of this type in canonical order; each call starts a fresh view.
    fn events<E: EventId<Time = S::Time> + 'static>(&self) -> impl Iterator<Item = &E>;
}
// Retain a clonable iterator of borrowed lanes, never cloned payloads.
struct Batch<'e, S: SnapshotApply, I: 'e, B> {
    id: u128,
    time: S::Time,
    count: u64,
    events: B,
    input: PhantomData<&'e I>,
    snapshot: PhantomData<fn() -> S>,
}
impl<'e, S: SnapshotApply, I: 'e, B> Batch<'e, S, I, B> {
    fn new(id: u128, time: S::Time, count: u64, events: B) -> Self {
        Self { id, time, count, events, input: PhantomData, snapshot: PhantomData }
    }
}
impl<'e, S: SnapshotApply, I: EventSource + 'e, B: Iterator<Item = &'e I> + Clone> ApplyContext<S> for Batch<'e, S, I, B> {
    fn snapshot_id(&self) -> u128 {
        self.id
    }
    fn time(&self) -> &S::Time {
        &self.time
    }
    fn history_event_count(&self) -> u64 {
        self.count
    }
    fn events<E: EventId<Time = S::Time> + 'static>(&self) -> impl Iterator<Item = &E> {
        self.events.clone().filter_map(|event| event.get::<E>())
    }
}
/// Bridges the low-level one-pass storage iterator to repeatable borrowed views.
/// Payloads are never copied; buckets larger than eight allocate reference storage.
// Keep the concrete iterator visible through both bridge layers. An out-of-line
// call here leaves virtual Iterator::next calls even for a known slice iterator.
#[doc(hidden)]
#[inline(always)]
pub fn apply_batch<S: SnapshotApply, I: EventSource>(snapshot: &mut S, raw: checkpoints::ApplyBatch<'_, '_, S::Time, I>) {
    // Most histories have one input per timestamp. Keep that case as a single
    // reference, avoiding array initialization and buffered-iterator machinery.
    let Some(first) = raw.events.next() else {
        return snapshot.apply_batch(Batch::<S, I, _>::new(raw.snapshot_id, raw.time, raw.history_event_count, std::iter::empty::<&I>()));
    };
    let Some(second) = raw.events.next() else {
        return snapshot.apply_batch(Batch::<S, I, _>::new(raw.snapshot_id, raw.time, raw.history_event_count, std::iter::once(first)));
    };
    let mut inline = [None; 8];
    inline[0] = Some(first);
    inline[1] = Some(second);
    let mut len = 2;
    let mut overflow = Vec::new();
    for event in raw.events {
        if len < inline.len() {
            inline[len] = Some(event);
            len += 1;
        } else {
            overflow.push(event);
        }
    }
    let events = inline[..len].iter().copied().flatten().chain(overflow.iter().copied());
    snapshot.apply_batch(Batch::<S, I, _>::new(raw.snapshot_id, raw.time, raw.history_event_count, events));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contime_core::module::EventSet;
    use std::rc::Rc;
    struct Changed {
        value: Rc<u64>,
    }
    struct Other;
    impl checkpoints::Event for Changed {
        type Time = i64;
        fn time(&self) -> i64 {
            -5
        }
    }
    impl EventId for Changed {
        fn event_id(&self) -> u128 {
            *self.value as u128
        }
    }
    #[derive(Clone, Default)]
    struct State {
        time: i64,
        seen: usize,
    }
    impl checkpoints::Snapshot for State {
        type Time = i64;
        fn time(&self) -> &i64 {
            &self.time
        }
        fn set_time(&mut self, time: i64) {
            self.time = time;
        }
    }
    impl SnapshotId for State {
        fn snapshot_id(&self) -> u128 {
            7
        }
    }
    impl SnapshotApply for State {
        fn define(events: &mut EventSetBuilder<Self>) {
            events.event::<Changed>(|_, emit| emit(7));
        }
        fn create(_: u128, _: EventRef<'_>) -> Self {
            Self::default()
        }
        fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
            assert_eq!((batch.snapshot_id(), *batch.time(), batch.history_event_count()), (7, -5, 42));
            let first = batch.events::<Changed>().collect::<Vec<_>>();
            let second = batch.events::<Changed>().collect::<Vec<_>>();
            for (a, b) in first.iter().zip(&second) {
                assert!(std::ptr::eq(*a, *b));
                assert_eq!(Rc::strong_count(&a.value), 1);
            }
            self.seen = first.len();
        }
    }
    #[test]
    fn typed_views_repeat_without_cloning_thread_local_payloads_for_inline_and_spilled_buckets() {
        for count in [0, 1, 8, 9, 64] {
            let events = (0..count).map(|i| Changed { value: Rc::new(i) }).collect::<Vec<_>>();
            let mut iterator = events.iter();
            let mut state = State::default();
            apply_batch(&mut state, checkpoints::ApplyBatch { snapshot_id: 7, time: -5, history_event_count: 42, events: &mut iterator });
            assert_eq!(state.seen, count as usize);
        }
    }
    #[test]
    fn routing_checks_type_and_destination_before_initializing() {
        let event = Changed { value: Rc::new(1) };
        let set = EventSet::<State>::new();
        assert!(EventRef::new(&event).get::<Other>().is_none());
        assert!(set.create(7, EventRef::new(&event)).is_some());
        assert!(set.create(9, EventRef::new(&event)).is_none());
        assert!(set.create(7, EventRef::new(&Other)).is_none());
    }
}
