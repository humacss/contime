use contime::{ApplyBatch, ApplyEvents, Event, Snapshot, SnapshotEvent};

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

impl From<OnFirst> for FirstAtEvent {
    fn from(event: OnFirst) -> Self { Self::Shared(event) }
}

impl From<OnSecond> for SecondAtEvent {
    fn from(event: OnSecond) -> Self { Self::Shared(event) }
}

impl SnapshotEvent<FirstAt> for OnFirst {
    fn snapshot_id(&self) -> u128 { self.id }
}

impl SnapshotEvent<SecondAt> for OnSecond {
    fn snapshot_id(&self) -> u128 { self.id }
}

impl SnapshotEvent<FirstAt> for FirstAtEvent {
    fn snapshot_id(&self) -> u128 {
        match self {
            FirstAtEvent::Shared(event) => <OnFirst as SnapshotEvent<FirstAt>>::snapshot_id(event),
        }
    }
}

impl SnapshotEvent<SecondAt> for SecondAtEvent {
    fn snapshot_id(&self) -> u128 {
        match self {
            SecondAtEvent::Shared(event) => <OnSecond as SnapshotEvent<SecondAt>>::snapshot_id(event),
        }
    }
}

impl ApplyEvents for FirstAt {
    fn apply_events(&mut self, batch: ApplyBatch<'_, Self::Event>) {
        for event in batch.events.iter().copied() {
            match event {
                FirstAtEvent::Shared(event) => self.id = event.id,
            }
        }
        self.time = batch.time;
    }
}

impl ApplyEvents for SecondAt {
    fn apply_events(&mut self, batch: ApplyBatch<'_, Self::Event>) {
        for event in batch.events.iter().copied() {
            match event {
                SecondAtEvent::Shared(event) => self.id = event.id,
            }
        }
        self.time = batch.time;
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
