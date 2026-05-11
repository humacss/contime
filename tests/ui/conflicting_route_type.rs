use contime::{ApplyEvent, Event, Snapshot};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct FirstAt {
    id: u128,
    time: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SecondAt {
    id: u128,
    time: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OnFirst {
    event_id: u128,
    time: i64,
    id: u128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OnSecond {
    event_id: u128,
    time: i64,
    id: u128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum FirstAtEvent {
    Shared(OnFirst),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SecondAtEvent {
    Shared(OnSecond),
}

impl Snapshot for FirstAt {
    type Event = FirstAtEvent;

    fn id(&self) -> u128 { self.id }
    fn time(&self) -> i64 { self.time }
    fn set_time(&mut self, time: i64) { self.time = time; }
    fn conservative_size(&self) -> u64 { std::mem::size_of::<Self>() as u64 }
    fn from_event(event: &Self::Event) -> Self {
        match event {
            FirstAtEvent::Shared(event) => Self { id: event.id, time: event.time },
        }
    }
}

impl Snapshot for SecondAt {
    type Event = SecondAtEvent;

    fn id(&self) -> u128 { self.id }
    fn time(&self) -> i64 { self.time }
    fn set_time(&mut self, time: i64) { self.time = time; }
    fn conservative_size(&self) -> u64 { std::mem::size_of::<Self>() as u64 }
    fn from_event(event: &Self::Event) -> Self {
        match event {
            SecondAtEvent::Shared(event) => Self { id: event.id, time: event.time },
        }
    }
}

impl Event for OnFirst {
    fn id(&self) -> u128 { self.event_id }
    fn time(&self) -> i64 { self.time }
    fn conservative_size(&self) -> u64 { std::mem::size_of::<Self>() as u64 }
}

impl Event for OnSecond {
    fn id(&self) -> u128 { self.event_id }
    fn time(&self) -> i64 { self.time }
    fn conservative_size(&self) -> u64 { std::mem::size_of::<Self>() as u64 }
}

impl Event for FirstAtEvent {
    fn id(&self) -> u128 {
        match self {
            FirstAtEvent::Shared(event) => event.id(),
        }
    }

    fn time(&self) -> i64 {
        match self {
            FirstAtEvent::Shared(event) => event.time(),
        }
    }

    fn conservative_size(&self) -> u64 { std::mem::size_of::<Self>() as u64 }
}

impl Event for SecondAtEvent {
    fn id(&self) -> u128 {
        match self {
            SecondAtEvent::Shared(event) => event.id(),
        }
    }

    fn time(&self) -> i64 {
        match self {
            SecondAtEvent::Shared(event) => event.time(),
        }
    }

    fn conservative_size(&self) -> u64 { std::mem::size_of::<Self>() as u64 }
}

impl ApplyEvent<FirstAt> for OnFirst {
    fn snapshot_id(&self) -> u128 { self.id }
    fn apply_to(&self, snapshot: &mut FirstAt) {
        snapshot.id = self.id;
        snapshot.time = self.time;
    }
}

impl ApplyEvent<SecondAt> for OnSecond {
    fn snapshot_id(&self) -> u128 { self.id }
    fn apply_to(&self, snapshot: &mut SecondAt) {
        snapshot.id = self.id;
        snapshot.time = self.time;
    }
}

impl ApplyEvent<FirstAt> for FirstAtEvent {
    fn snapshot_id(&self) -> u128 {
        match self {
            FirstAtEvent::Shared(event) => event.snapshot_id(),
        }
    }

    fn apply_to(&self, snapshot: &mut FirstAt) {
        match self {
            FirstAtEvent::Shared(event) => event.apply_to(snapshot),
        }
    }
}

impl ApplyEvent<SecondAt> for SecondAtEvent {
    fn snapshot_id(&self) -> u128 {
        match self {
            SecondAtEvent::Shared(event) => event.snapshot_id(),
        }
    }

    fn apply_to(&self, snapshot: &mut SecondAt) {
        match self {
            SecondAtEvent::Shared(event) => event.apply_to(snapshot),
        }
    }
}

contime::fragment! {
    macro first_fragment;
    snapshots { FirstAt, }
    routes { Shared(OnFirst) => [FirstAt], }
}

contime::fragment! {
    macro second_fragment;
    snapshots { SecondAt, }
    routes { Shared(OnSecond) => [SecondAt], }
}

contime::lanes! {
    mod broken_lanes;
    fragments [
        first_fragment,
        second_fragment,
    ];
}

fn main() {}
