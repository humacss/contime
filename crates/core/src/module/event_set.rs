//! Registration and borrowed runtime event views; payloads stay in their lanes.
use super::{EventId, SnapshotApply};
use std::{
    any::{type_name, Any, TypeId},
    marker::PhantomData,
};

/// A borrowed event payload. The payload's type is static; the borrow is not.
#[derive(Clone, Copy)]
pub struct EventRef<'a>(&'a dyn Any);
impl<'a> EventRef<'a> {
    /// Borrows an event without cloning or boxing it.
    pub fn new<E: Any>(event: &'a E) -> Self {
        Self(event)
    }
    /// Reads a concrete payload, or `None` for another event type.
    pub fn get<E: Any>(&self) -> Option<&'a E> {
        self.0.downcast_ref()
    }
    #[inline]
    fn type_id(self) -> TypeId {
        self.0.type_id()
    }
}
/// Adapter from a concrete event or generated aggregate lane to its payload.
pub trait EventSource {
    /// Borrows the active concrete event.
    fn event_ref(&self) -> EventRef<'_>;
    /// Borrows one concrete type without consuming the aggregate lane.
    /// Generated adapters specialize this match so typed views need no virtual lookup.
    fn get<E: Any>(&self) -> Option<&E> {
        self.event_ref().get::<E>()
    }
    /// Reports whether this lane can contain the given event type.
    fn contains_event_type(id: TypeId) -> bool;
}
impl<E: EventId + 'static> EventSource for E {
    fn event_ref(&self) -> EventRef<'_> {
        EventRef::new(self)
    }
    fn contains_event_type(id: TypeId) -> bool {
        id == TypeId::of::<E>()
    }
}
type Routing = dyn Fn(EventRef<'_>, &mut dyn FnMut(u128)) + Send + Sync;
struct Registration {
    id: TypeId,
    name: &'static str,
    route: Box<Routing>,
}
/// Defines the accepted event types once, independently of application logic.
pub struct EventSetBuilder<S> {
    entries: Vec<Registration>,
    marker: PhantomData<fn() -> S>,
}
impl<S: SnapshotApply> EventSetBuilder<S> {
    /// Registers an event and the IDs it targets. Repeated registrations keep the first route.
    /// The callback emits zero, one or multiple snapshot IDs without creating state.
    pub fn event<E: EventId<Time = S::Time> + 'static>(
        &mut self,
        route: impl Fn(&E, &mut dyn FnMut(u128)) + Send + Sync + 'static,
    ) -> &mut Self {
        let id = TypeId::of::<E>();
        if !self.entries.iter().any(|entry| entry.id == id) {
            self.entries.push(Registration {
                id,
                name: type_name::<E>(),
                route: Box::new(move |event, emit| {
                    if let Some(event) = event.get::<E>() {
                        route(event, emit);
                    }
                }),
            });
        }
        self
    }
}
/// Immutable event registration shared by routing, creation and application.
/// Generated modules cache this once per snapshot type; concrete adapters can do the same.
pub struct EventSet<S> {
    entries: Vec<Registration>,
    marker: PhantomData<fn() -> S>,
}
impl<S: SnapshotApply> Default for EventSet<S> {
    fn default() -> Self {
        Self::new()
    }
}
impl<S: SnapshotApply> EventSet<S> {
    /// Builds a definition by calling `SnapshotApply::define` once.
    pub fn new() -> Self {
        let mut builder = EventSetBuilder { entries: Vec::new(), marker: PhantomData };
        S::define(&mut builder);
        Self { entries: builder.entries, marker: PhantomData }
    }
    /// Checks that every accepted event is available in the aggregate input lane.
    pub fn validate<I: EventSource>(&self) {
        for entry in &self.entries {
            assert!(
                I::contains_event_type(entry.id),
                "snapshot {} registers event {} absent from the module",
                type_name::<S>(),
                entry.name
            );
        }
    }
    /// Visits destinations for this snapshot type without initializing state.
    pub fn snapshot_ids(&self, event: EventRef<'_>, emit: &mut impl FnMut(u128)) {
        let id = event.type_id();
        if let Some(entry) = self.entries.iter().find(|entry| entry.id == id) {
            (entry.route)(event, emit);
        }
    }
    /// Initializes only an identity emitted by this event's registered route.
    pub fn create(&self, id: u128, event: EventRef<'_>) -> Option<S> {
        let mut targeted = false;
        self.snapshot_ids(event, &mut |candidate| targeted |= candidate == id);
        if !targeted {
            return None;
        }
        let snapshot = S::create(id, event);
        assert_eq!(snapshot.snapshot_id(), id, "created snapshot identity must match its event route");
        Some(snapshot)
    }
}
