#![allow(dead_code)]
use contime::{
    checkpoints::{Event, Snapshot},
    *,
};
use std::{num::NonZeroUsize, time::Duration};
pub const COUNTER: u128 = 1 << 127;

// No ConTime derives, no conventional `time` field, and no Clone on the events.
pub struct Added {
    pub at: i64,
    pub id: u128,
    pub lists: Vec<u128>,
    pub value: char,
}
impl Event for Added {
    type Time = i64;
    fn time(&self) -> i64 {
        self.at
    }
}
impl EventId for Added {
    fn event_id(&self) -> u128 {
        self.id
    }
}
pub struct Cleared {
    pub at: i64,
    pub id: u128,
    pub list: u128,
}
impl Event for Cleared {
    type Time = i64;
    fn time(&self) -> i64 {
        self.at
    }
}
impl EventId for Cleared {
    fn event_id(&self) -> u128 {
        self.id
    }
}
#[derive(Clone, Default)]
pub struct OrderedList {
    pub at: i64,
    pub id: u128,
    pub values: Vec<char>,
}
impl Snapshot for OrderedList {
    type Time = i64;
    fn time(&self) -> &i64 {
        &self.at
    }
    fn set_time(&mut self, time: i64) {
        self.at = time;
    }
}
impl SnapshotId for OrderedList {
    fn snapshot_id(&self) -> u128 {
        self.id
    }
}
impl SnapshotApply for OrderedList {
    fn define(events: &mut EventSetBuilder<Self>) {
        events
            .event::<Added>(|event, emit| {
                for &id in &event.lists {
                    emit(id);
                }
            })
            .event::<Cleared>(|event, emit| emit(event.list));
    }
    fn create(id: u128, _: EventRef<'_>) -> Self {
        Self { id, ..Self::default() }
    }
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        assert_eq!(batch.snapshot_id(), self.id);
        assert_eq!(batch.events::<Added>().count(), batch.events::<Added>().count());
        for _ in batch.events::<Cleared>() {
            self.values.clear();
        }
        for event in batch.events::<Added>() {
            self.values.push(event.value);
        }
    }
}
#[derive(Clone, Default)]
pub struct Counter {
    pub at: i64,
    pub id: u128,
    pub count: u64,
}
impl Snapshot for Counter {
    type Time = i64;
    fn time(&self) -> &i64 {
        &self.at
    }
    fn set_time(&mut self, time: i64) {
        self.at = time;
    }
}
impl SnapshotId for Counter {
    fn snapshot_id(&self) -> u128 {
        self.id
    }
}
impl SnapshotApply for Counter {
    fn define(events: &mut EventSetBuilder<Self>) {
        events.event::<Added>(|event, emit| {
            for &id in &event.lists {
                emit(COUNTER | id);
            }
        });
    }
    fn create(id: u128, _: EventRef<'_>) -> Self {
        Self { id, ..Self::default() }
    }
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        self.count += batch.events::<Added>().count() as u64;
    }
}

#[derive(ContimeModule)]
#[contime_module(time=i64,events=[Added, Cleared, Added],snapshots=[OrderedList])]
pub struct Shared;
#[derive(ContimeModule)]
#[contime_module(time=i64,events=[Added],snapshots=[Counter],modules=[Shared])]
pub struct Statistics;
#[derive(ContimeModule)]
#[contime_module(time=i64,events=[Added],snapshots=[OrderedList],modules=[Shared,Statistics,Shared])]
pub struct App;
// Explicit names let differently named imports contribute the same registration.
use Added as Incoming;
use OrderedList as List;
#[derive(ContimeModule)]
#[contime_module(time=i64,events=[Added=Incoming],snapshots=[OrderedList=List],modules=[Shared])]
pub struct Aliases;
pub fn config() -> ConTimeConfig<i64> {
    ConTimeConfig {
        router_count: 1,
        worker_count: 2,
        placement: Placement::default(),
        pruning_interval: Duration::from_secs(60),
        history_retention: 60000,
        checkpoints: checkpoints::CheckpointConfig { interval: 2 },
    }
}
pub fn manual_config() -> manual::ManualConfig {
    manual::ManualConfig { queue_capacity: NonZeroUsize::new(32).unwrap(), checkpoints: checkpoints::CheckpointConfig { interval: 2 } }
}
pub fn added(at: i64, id: u128, value: char) -> Added {
    Added { at, id, lists: vec![7, 9], value }
}
