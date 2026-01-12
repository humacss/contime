mod api;
mod apply;
mod flume_ext;
mod history;
mod index;
mod key;
mod router;
mod traits;
mod worker;

use apply::{apply_event_in_place};
use history::{Checkpoint, SnapshotHistory};
use index::{index_before, indexes_between};
use key::{ContimeKey};
use router::{Router, RouterError};
use traits::{SnapshotLanes, EventLanes};
use worker::{Worker, WorkerInbound, WorkerOutbound};
use flume_ext::{SnapshotReceiver};

pub use api::{Contime, ContimeError};
pub use traits::{Snapshot, Event, ApplyEvent};

// Enable this once we have the macro
//#[cfg(test)]
mod test;
// Enable this once we have the macro
//#[cfg(test)]
pub use test::*;
