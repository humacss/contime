use crate::{TestEvent, TestSnapshot};

crate::lanes! {
    mod __contime;
    snapshots [TestSnapshot];
    routes [
        TestEvent => [TestSnapshot],
    ];
}

pub use __contime::Contime as TestSnapshotContime;
pub use __contime::InputLanes as TestInputLanes;
pub use __contime::SnapshotLanes as TestSnapshotLanes;
