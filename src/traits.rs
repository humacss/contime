use std::fmt::Debug;

// we're going to create a composite ourselves, because that can be used for both snapshots and events
// if we depend on a key provided by the user it won't work cleanly for snapshots
pub trait Event: Send + Sync + Debug {
    fn id(&self) -> u128;
    fn time(&self) -> i64;
}

pub trait ApplyEvent<S>: Event + Debug
where
    S: Snapshot,
{
    fn snapshot_id(&self) -> u128;
    fn apply_to(&self, snapshot: &mut S);
}

pub trait Snapshot: Default + Send + Sync + Clone + Debug + PartialEq + Eq {
    type Event: Event + ApplyEvent<Self>;

    fn id(&self) -> u128;
    fn time(&self) -> i64;
    fn set_time(&mut self, time: i64);
}
