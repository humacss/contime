
pub use crate::{Event, Snapshot};

#[derive(Default, Debug, Clone, PartialEq, PartialOrd, Ord, Eq)]
pub struct ContimeKey {
	pub time: i64,
    pub id: u128,
}

impl ContimeKey {
    pub fn from_event<E: Event>(event: &E) -> ContimeKey{
        ContimeKey { id: event.id(), time: event.time() }
    }

    pub fn from_snapshot<S: Snapshot>(snapshot: &S) -> ContimeKey{
        ContimeKey { id: snapshot.id(), time: snapshot.time() }
    }
}
