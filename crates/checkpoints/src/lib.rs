//! Apply-time snapshot replay and checkpoint storage independent of ConTime
//! orchestration.

mod advance;
mod apply;
mod checkpoints;
mod query;
mod replay;
mod types;

pub use advance::advance_before;
pub use apply::apply;
pub use query::query_at;
pub use replay::replay;
pub use types::{
    AdvanceResult, ApplyBatch, ApplyEvents, ApplyInner, ApplyResult, ApplyWrapper, Checkpoint, CheckpointConfig, CheckpointKey,
    CheckpointStore, EventBatch, EventRef, Events, ReplayAnchor, Snapshot,
};
