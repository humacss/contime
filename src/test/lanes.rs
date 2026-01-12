// =============================================================================
// TEMPORARY - Manual lane enums for testing/development
//
// These will be completely replaced by the contime!{} macro that generates
// proper enum variants + all boilerplate implementations.
// =============================================================================

use crate::{TestSnapshot, TestEvent, SnapshotLanes, Snapshot, Event, ApplyEvent, EventLanes};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TestSnapshotLanes {
    TestSnapshot(TestSnapshot),
}

impl SnapshotLanes for TestSnapshotLanes {}

impl Snapshot for TestSnapshotLanes {
    type Event = TestEventLanes;

    fn id(&self) -> u128 {
        match self {
            Self::TestSnapshot(snapshot) => snapshot.id()
        }
    }
    fn time(&self) -> i64 {
        match self {
            Self::TestSnapshot(snapshot) => snapshot.time()
        }
    }

    fn set_time(&mut self, time: i64) {
        match self {
            Self::TestSnapshot(snapshot) => snapshot.time = time
        }
    }

    fn conservative_size(&self) -> usize {
        match self {
            Self::TestSnapshot(snapshot) => snapshot.conservative_size()
        }
    }
    
    fn from_event(event: &Self::Event) -> Self { 
        match event {
            TestEventLanes::TestEvent(event) => Self::TestSnapshot(TestSnapshot { id: event.snapshot_id(), time: event.time(), items: vec![], sum: 0 })
        }
    }
}

impl From<TestSnapshot> for TestSnapshotLanes {
    fn from(snapshot: TestSnapshot) -> Self {
        Self::TestSnapshot(snapshot)
    }
}

impl From<TestSnapshotLanes> for TestSnapshot {
    fn from(snapshot_lane: TestSnapshotLanes) -> Self {
        match snapshot_lane {
            TestSnapshotLanes::TestSnapshot(snapshot) => {
                return snapshot;
            }
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum TestEventLanes {
    TestEvent(TestEvent),
}

impl Event for TestEventLanes {
    fn id(&self) -> u128 {
        match self {
            Self::TestEvent(event) => event.id()
        }
    }
    fn time(&self) -> i64 {
        match self {
            Self::TestEvent(event) => event.time()
        }
    }

    fn conservative_size(&self) -> usize { 
        match self {
            Self::TestEvent(event) => event.conservative_size()
        }
    }  
}

impl ApplyEvent<TestSnapshotLanes> for TestEventLanes {
    fn snapshot_id(&self) -> u128 {
        match self {
            TestEventLanes::TestEvent(event) => event.snapshot_id(),
        }
    }

    fn apply_to(&self, snapshot: &mut TestSnapshotLanes) -> isize {
        match self {
            Self::TestEvent(event) => {
                match snapshot {
                    TestSnapshotLanes::TestSnapshot(snapshot) => event.apply_to(snapshot),
                }
            }
        }
    }
    
    fn conservative_apply_size_delta(&self) -> isize {
        match self {
            Self::TestEvent(event) => event.conservative_apply_size_delta()
        }
    }
}

impl<SL: SnapshotLanes + std::convert::From<TestSnapshot>> EventLanes<SL> for TestEventLanes where TestEventLanes: ApplyEvent<SL> {
    fn snapshots(&self) -> Vec<SL> { 
        match self {
            Self::TestEvent(event) => {
                vec![TestSnapshot::from_event(event).into()]
            },
        } 
    }
}

impl From<TestEvent> for TestEventLanes {
    fn from(event: TestEvent) -> Self {
        Self::TestEvent(event)
    }
}

impl From<TestEvent> for TestSnapshot {
    fn from(event: TestEvent) -> Self {
        let mut snapshot = Self::default();

        snapshot.id = event.snapshot_id();

        snapshot
    }
}