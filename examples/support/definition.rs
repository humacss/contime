//! Host-independent example contracts. No generator, thread, terminal, or browser APIs.
use contime::checkpoints::{ApplyEvents, Snapshot, Timestamp};
use contime::{AdvanceTime, Input};

/// Live hosts use nanoseconds; controlled verification loads use logical ticks.
/// These values are application time, not wall-clock timestamps.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub struct Time(pub u64);
impl std::fmt::Display for Time {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}ns", self.0)
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
/// Domain hooks used by shared example drivers. ConTime itself needs only Input
/// and ApplyEvents; these extra hooks supply synthetic inputs and display data.
#[allow(dead_code)] // Hosts select case hooks; the launcher itself constructs no model.
pub trait Example: 'static {
    type Event: Input<Time = Time> + Clone;
    type State: Snapshot<Time = Time> + ApplyEvents<Self::Event> + Clone + Default;
    const TITLE: &'static str;
    const VALUE_LABEL: &'static str;
    #[allow(dead_code)] // Finite portable verification constructs model-specific events.
    fn event(id: u128, time: Time, targets: Vec<u128>, value: i64) -> Self::Event;
    fn id(state: &Self::State) -> u128;
    fn count(state: &Self::State) -> u64;
    fn value(state: &Self::State) -> i64;
    /// Independent reference application for correctness checks.
    fn record(state: &mut Self::State, event: &Self::Event);
    fn hash(_state: &Self::State) -> Option<[u8; 32]> {
        None
    }
}
