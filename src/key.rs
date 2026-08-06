pub use crate::{ContimeTime, Input, Snapshot};

#[derive(Default, Debug, Clone, PartialEq, PartialOrd, Ord, Eq)]
pub struct ContimeKey<T: ContimeTime> {
    pub time: T,
    pub id: u128,
}

impl<T: ContimeTime> ContimeKey<T> {
    pub fn from_input<I: Input<Time = T>>(input: &I) -> Self {
        ContimeKey { id: input.id(), time: input.time() }
    }

    pub fn from_snapshot<S: Snapshot<Time = T>>(snapshot: &S) -> Self {
        ContimeKey { id: snapshot.id(), time: snapshot.time() }
    }
}
