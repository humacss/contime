//! Minimal lane contracts for the worker renderer integration test.
use contime::checkpoints::Timestamp;
use contime::{AdvanceTime, ContimeEvent, ContimeSnapshot};
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub struct Time(pub u64);
impl std::fmt::Display for Time {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}ms", self.0)
    }
}
impl Timestamp for Time {
    fn minimum() -> Self {
        Self(0)
    }
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}
impl AdvanceTime for Time {
    fn is_valid_retention(&self) -> bool {
        true
    }
    fn saturating_sub(&self, other: &Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }
}
#[derive(ContimeEvent)]
#[contime_event(id = |event| event.id)]
pub struct Increment {
    pub id: u128,
    pub time: Time,
}
#[derive(ContimeSnapshot)]
#[contime_snapshot(id = |id| *id, events = [Increment => [self.id % 4096]], apply = {
    self.count += batch.events.count() as u64;
})]
pub struct Counter {
    id: u128,
    time: Time,
    count: u64,
}

#[path = "../../../lanes.rs"]
mod lane_adapter;
lane_adapter::single_lane!(Increment => Counter);
