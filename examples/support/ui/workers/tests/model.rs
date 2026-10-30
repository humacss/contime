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

pub struct Increment {
    pub id: u128,
    pub time: Time,
}
impl contime::checkpoints::Event for Increment {
    type Time = Time;
    fn time(&self) -> Self::Time {
        self.time
    }
}

impl contime::EventId for Increment {
    fn event_id(&self) -> u128 {
        self.id
    }
}

#[derive(Clone, Default)]

pub struct Counter {
    id: u128,
    time: Time,
    count: u64,
}
impl contime::checkpoints::Snapshot for Counter {
    type Time = Time;
    fn time(&self) -> &Self::Time {
        &self.time
    }
    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }
}

impl Counter {
    pub fn id_for(id: &u128) -> u128 {
        *id
    }
}
impl contime::SnapshotId for Counter {
    fn snapshot_id(&self) -> u128 {
        Self::id_for(&self.id)
    }
}
impl contime::SnapshotApply for Counter {
    fn define(events: &mut contime::EventSetBuilder<Self>) {
        events.event::<Increment>(|event, emit| {
            emit(<Counter>::id_for(&(event.id % 4096)));
        });
    }
    fn create(snapshot_id: u128, _event: contime::EventRef<'_>) -> Self {
        Self { id: snapshot_id, ..Self::default() }
    }
    fn apply_batch(&mut self, batch: impl contime::ApplyContext<Self>) {
        self.count += batch.events::<Increment>().count() as u64;
    }
}

#[path = "../../../lanes.rs"]
mod lane_adapter;
lane_adapter::single_lane!(Increment => Counter);
