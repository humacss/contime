//! Ordinary trait bounds need not be repeated on data declarations.
use contime::prelude::*;
use std::marker::PhantomData;

struct BoundedEvent<T> {
    time: i64,
    owner: u128,
    value: T,
}
impl<T> contime::checkpoints::Event for BoundedEvent<T> {
    type Time = i64;
    fn time(&self) -> Self::Time {
        self.time
    }
}

impl<T: Copy + Into<u128>> EventId for BoundedEvent<T> {
    fn event_id(&self) -> u128 {
        self.value.into()
    }
}

struct BoundedState<T> {
    time: i64,
    owner: u128,
    values: Vec<u128>,
    marker: PhantomData<T>,
}
impl<T> Clone for BoundedState<T> {
    fn clone(&self) -> Self {
        Self { time: self.time, owner: self.owner, values: self.values.clone(), marker: self.marker }
    }
}
impl<T> Default for BoundedState<T> {
    fn default() -> Self {
        Self {
            time: ::core::default::Default::default(),
            owner: ::core::default::Default::default(),
            values: ::core::default::Default::default(),
            marker: ::core::default::Default::default(),
        }
    }
}
impl<T> contime::checkpoints::Snapshot for BoundedState<T> {
    type Time = i64;
    fn time(&self) -> &Self::Time {
        &self.time
    }
    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }
}

impl<T: Copy + Into<u128>> SnapshotId for BoundedState<T> {
    fn snapshot_id(&self) -> u128 {
        self.owner
    }
}

impl<T: Copy + Into<u128> + 'static> SnapshotApply for BoundedState<T> {
    fn define(events: &mut contime::EventSetBuilder<Self>) {
        events.event::<BoundedEvent<T>>(|event, emit| {
            emit(event.owner);
        });
    }
    fn create(snapshot_id: u128, event: contime::EventRef<'_>) -> Self {
        if let Some(event) = event.get::<BoundedEvent<T>>() {
            let mut snapshot = Self::default();
            {
                snapshot.owner = event.owner;
                if contime::SnapshotId::snapshot_id(&snapshot) == snapshot_id {
                    return snapshot;
                }
            }
        }
        panic!("event does not initialize the requested snapshot")
    }
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        for event in batch.events::<BoundedEvent<T>>() {
            self.values.push(event.value.into());
        }
    }
}
#[derive(ContimeModule)]
#[contime_module(time=i64,events=[BoundedEvent<u64>],snapshots=[BoundedState<u64>])]
struct App;
#[test]
fn implementation_bounds_need_not_be_repeated_on_event_or_snapshot_data() {
    struct NoIdentity;
    let _ = BoundedState::<NoIdentity>::default();
    let event = BoundedEvent { time: -10, owner: 7, value: 3u64 };
    assert_eq!(event.event_id(), 3);
    let contime = ConTime::<App>::start(crate::config()).unwrap();
    contime.apply([event.into()]).unwrap();
    contime.wait_until_idle(std::time::Duration::from_secs(3)).unwrap();
    let snapshots = contime.query_at(-10, [7]).unwrap();
    let AppSnapshotLanes::BoundedState(snapshot) = &*snapshots[0];
    assert_eq!(snapshot.snapshot_id(), 7);
    assert_eq!(snapshot.values, [3]);
    assert!(contime.errors().is_empty());
    contime.shutdown();
}
