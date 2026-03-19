use std::fmt::Debug;

pub trait Snapshot: Send + Sync + Clone + Debug + PartialEq + Eq {
    type Event: Event + ApplyEvent<Self>;

    fn id(&self) -> u128;
    fn time(&self) -> i64;
    fn set_time(&mut self, time: i64);
    fn conservative_size(&self) -> u64;
    fn from_event(event: &Self::Event) -> Self;
}

pub trait SnapshotLanes: Snapshot {}

pub trait Event: Send + Sync + Debug {
    fn id(&self) -> u128;
    fn time(&self) -> i64;
    fn conservative_size(&self) -> u64;
}

pub trait ApplyEvent<S>: Event
where
    S: Snapshot,
{
    fn snapshot_id(&self) -> u128;
    fn apply_to(&self, snapshot: &mut S);
}

pub trait EventLanes<SL: SnapshotLanes>: Event + ApplyEvent<SL> + Clone {
    fn snapshots(&self) -> Vec<SL>;
}
