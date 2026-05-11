use contime::{ApplyEvent, Event, Snapshot};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct BrokenAt {
    id: u128,
    time: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OnBroken {
    event_id: u128,
    time: i64,
    id: u128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum BrokenAtEvent {
    OnBroken(OnBroken),
}

impl Snapshot for BrokenAt {
    type Event = BrokenAtEvent;

    fn id(&self) -> u128 { self.id }
    fn time(&self) -> i64 { self.time }
    fn set_time(&mut self, time: i64) { self.time = time; }
    fn conservative_size(&self) -> u64 { std::mem::size_of::<Self>() as u64 }
    fn from_event(event: &Self::Event) -> Self {
        match event {
            BrokenAtEvent::OnBroken(event) => Self { id: event.id, time: event.time },
        }
    }
}

impl Event for OnBroken {
    fn id(&self) -> u128 { self.event_id }
    fn time(&self) -> i64 { self.time }
    fn conservative_size(&self) -> u64 { std::mem::size_of::<Self>() as u64 }
}

impl Event for BrokenAtEvent {
    fn id(&self) -> u128 {
        match self {
            BrokenAtEvent::OnBroken(event) => event.id(),
        }
    }

    fn time(&self) -> i64 {
        match self {
            BrokenAtEvent::OnBroken(event) => event.time(),
        }
    }

    fn conservative_size(&self) -> u64 { std::mem::size_of::<Self>() as u64 }
}

impl ApplyEvent<BrokenAt> for OnBroken {
    fn snapshot_id(&self) -> u128 { self.id }
    fn apply_to(&self, snapshot: &mut BrokenAt) {
        snapshot.id = self.id;
        snapshot.time = self.time;
    }
}

impl ApplyEvent<BrokenAt> for BrokenAtEvent {
    fn snapshot_id(&self) -> u128 {
        match self {
            BrokenAtEvent::OnBroken(event) => event.snapshot_id(),
        }
    }

    fn apply_to(&self, snapshot: &mut BrokenAt) {
        match self {
            BrokenAtEvent::OnBroken(event) => event.apply_to(snapshot),
        }
    }
}

contime::fragment! {
    macro broken_fragment;
    snapshots { BrokenAt, }
    routes { Shared(OnBroken) => BrokenAt, }
}

fn main() {}
