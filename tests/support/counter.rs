// Feature-only builds use different subsets of the shared domain fixture.
#![allow(dead_code)]
#[cfg(any(feature = "runtime", feature = "manual"))]
use contime::checkpoints::CheckpointConfig;
use contime::checkpoints::{ApplyBatch, ApplyEvents, Event, Snapshot, Timestamp};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub struct Time(pub u64);
impl Timestamp for Time {
    fn minimum() -> Self {
        Self(0)
    }
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}
impl contime::core::AdvanceTime for Time {
    fn is_valid_retention(&self) -> bool {
        true
    }
    fn saturating_sub(&self, other: &Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub id: u128,
    pub at: Time,
    pub targets: Vec<u128>,
    pub delta: i64,
}
impl Event for Change {
    type Time = Time;
    fn time(&self) -> Time {
        self.at
    }
}
impl contime::Input for Change {
    fn event_id(&self) -> u128 {
        self.id
    }
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        for &id in &self.targets {
            emit(id);
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct Counter {
    pub id: u128,
    pub at: Time,
    pub value: i64,
    pub order: Vec<u128>,
}
impl Snapshot for Counter {
    type Time = Time;
    fn time(&self) -> &Time {
        &self.at
    }
    fn set_time(&mut self, time: Time) {
        self.at = time;
    }
}
impl ApplyEvents<Change> for Counter {
    fn create(id: u128, _: &Change) -> Self {
        Self { id, ..Self::default() }
    }
    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Time, Change>) {
        for event in batch.events {
            self.value += event.delta;
            self.order.push(event.id);
        }
    }
}
#[cfg(feature = "runtime")]
pub fn runtime(workers: usize) -> contime::RawConTime<Change, Counter, ()> {
    contime::RawConTime::start(
        contime::ConTimeConfig {
            worker_count: workers,
            router_count: 1,
            placement: contime::Placement::default(),
            history_retention: Time(u64::MAX),
            pruning_interval: std::time::Duration::from_secs(60),
            checkpoints: CheckpointConfig { interval: 4 },
        },
        (),
    )
    .unwrap()
}
#[cfg(feature = "manual")]
pub fn manual() -> contime::manual::RawManualConTime<Change, Counter> {
    contime::manual::RawManualConTime::new(
        contime::manual::ManualConfig {
            queue_capacity: std::num::NonZeroUsize::new(64).unwrap(),
            checkpoints: CheckpointConfig { interval: 4 },
        },
        (),
    )
}
#[cfg(feature = "manual")]
pub fn drive(engine: &mut contime::manual::RawManualConTime<Change, Counter>) {
    for _ in 0..10_000 {
        if !engine.process().unwrap() {
            return;
        }
    }
    panic!("manual work did not settle");
}
