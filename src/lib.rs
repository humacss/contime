mod api;
mod apply;
mod handle;
mod history;
mod key;
mod router;
mod traits;
mod worker;
mod macros;

use apply::apply_event_in_place;
use key::ContimeKey;
use router::{Router, RouterError};
use worker::{Worker, WorkerInbound};

pub use history::{SnapshotHistory, Reconciliation};
pub use api::{Contime, ContimeError};
pub use traits::{Snapshot, Event, ApplyEvent, SnapshotLanes, EventLanes};
pub use handle::{ApplyHandle, QueryHandle, QueryResult, AdvanceHandle, HandleError};

mod test;
pub use test::*;
