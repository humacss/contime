//! Common application contracts, derives and execution entry points.
//! Only items selected by the package's enabled features are re-exported.
pub use crate::checkpoints::{CheckpointConfig, Timestamp};
#[cfg(feature = "manual")]
pub use crate::manual::{ManualConTime, ManualConfig};
#[cfg(feature = "macros")]
pub use crate::ContimeModule;
pub use crate::{AdvanceTime, Input};
pub use crate::{ApplyContext, EventId, EventRef, EventSetBuilder, Module, SnapshotApply, SnapshotId};
#[cfg(feature = "runtime")]
pub use crate::{ConTime, ConTimeConfig, Placement};
