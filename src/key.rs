use std::sync::Arc;
pub use crate::{Event, Snapshot};

#[derive(Clone, PartialEq, PartialOrd)]
pub struct ContimeKey {
	pub time: i64,
    pub id: u128,
}

impl ContimeKey {
    pub fn from_event<E: Event>(event: &Arc<E>) -> ContimeKey{
        ContimeKey { id: event.id(), time: event.time() }
    }

    pub fn from_snapshot<S: Snapshot>(snapshot: &S) -> ContimeKey{
        ContimeKey { id: snapshot.id(), time: snapshot.time() }
    }
}

impl Default for ContimeKey {
    fn default() -> Self { Self { id: 0, time: 0  } }
}