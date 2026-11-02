//! Minimal lane contracts for the worker renderer integration test.
use contime::checkpoints::Timestamp;
use contime::AdvanceTime;
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

#[derive(contime::ContimeEvent)]
pub struct Increment {
    #[contime(id)]
    pub id: u128,
    #[contime(time)]
    pub time: Time,
}

#[derive(Clone, Default, contime::ContimeSnapshot)]
#[contime(event = Increment, id = counter_event_ids)]
pub struct Counter {
    #[contime(id)]
    id: u128,
    #[contime(time)]
    time: Time,
    count: u64,
}

fn counter_event_ids(event: &Increment, emit: &mut dyn FnMut(u128)) {
    emit(event.id % 4096);
}

// Routing folds event IDs into a bounded snapshot set, so creation uses the
// routed destination rather than copying the event's original ID.
impl contime::SnapshotCreate for Counter {
    fn create(snapshot_id: u128, _event: contime::EventRef<'_>) -> Self {
        Self { id: snapshot_id, ..Self::default() }
    }
}
impl contime::SnapshotApply for Counter {
    fn apply_batch(&mut self, batch: impl contime::ApplyContext<Self>) {
        self.count += batch.events::<Increment>().count() as u64;
    }
}

#[path = "../../../lanes.rs"]
mod lane_adapter;
lane_adapter::single_lane!(Increment => Counter);
