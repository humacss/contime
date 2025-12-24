pub mod apply;
pub mod history;
pub mod index;

pub use apply::apply_event_in_place;
pub use history::Checkpoint;
pub use index::{index_before, indexes_between};
pub use crate::{Event, Snapshot};

pub use history::SnapshotHistory;

#[derive(Clone, PartialEq, PartialOrd)]
pub struct ContimeKey {
	pub time: i64,
    pub id: u128,
}

impl ContimeKey {
    fn from_event<E: Event>(event: &E) -> ContimeKey{
        ContimeKey { id: event.id(), time: event.time() }
    }

    fn from_snapshot<S: Snapshot>(snapshot: &S) -> ContimeKey{
        ContimeKey { id: snapshot.id(), time: snapshot.time() }
    }
}

impl Default for ContimeKey {

fn default() -> Self { Self { id: 0, time: 0  } }
}