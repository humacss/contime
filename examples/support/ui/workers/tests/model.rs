//! Minimal lane contracts for the worker renderer integration test.
use contime_runtime::{
    checkpoints::{ApplyBatch, ApplyEvents, Event, Snapshot, Timestamp},
    AdvanceTime, Input,
};
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub struct Time(pub u64);
impl std::fmt::Display for Time {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}ms", self.0)
    }
}
impl Timestamp for Time {
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}
impl AdvanceTime for Time {
    fn saturating_sub(&self, other: &Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }
}
pub struct Increment {
    pub id: u128,
    pub time: Time,
}
impl Event for Increment {
    type Time = Time;
    fn time(&self) -> Time {
        self.time
    }
}
impl Input for Increment {
    fn event_id(&self) -> u128 {
        self.id
    }
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        emit(self.id % 4096);
    }
}
#[derive(Clone, Default)]
pub struct Counter {
    time: Time,
    count: u64,
}
impl Snapshot for Counter {
    type Time = Time;
    fn time(&self) -> &Time {
        &self.time
    }
    fn set_time(&mut self, time: Time) {
        self.time = time;
    }
}
impl ApplyEvents<Increment> for Counter {
    fn create(_: u128, _: &Increment) -> Self {
        Self::default()
    }
    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Time, Increment>) {
        self.count += batch.events.count() as u64;
    }
}
