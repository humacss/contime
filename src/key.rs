pub use crate::{ContimeTime, Event, Snapshot};

#[derive(Default, Debug, Clone, PartialEq, PartialOrd, Ord, Eq)]
pub struct ContimeKey<T: ContimeTime> {
    pub time: T,
    pub id: u128,
}

impl<T: ContimeTime> ContimeKey<T> {
    pub fn from_event<E: Event<Time = T>>(event: &E) -> Self {
        ContimeKey { id: event.id(), time: event.time() }
    }

    pub fn from_snapshot<S: Snapshot<Time = T>>(snapshot: &S) -> Self {
        ContimeKey { id: snapshot.id(), time: snapshot.time() }
    }
}
