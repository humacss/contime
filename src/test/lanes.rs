use crate::{TestEvent, TestSnapshot};

crate::contime! {
    TestSnapshot {
        TestEvent,
    }
}

pub use __contime::Contime as TestSnapshotContime;
pub use __contime::EventLanes as TestEventLanes;
pub use __contime::SnapshotLanes as TestSnapshotLanes;
