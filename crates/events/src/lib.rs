//! Canonical ownership-generic event storage independent of ConTime orchestration.

mod advance;
mod history;
mod insert;
mod iteration;
mod query;
mod types;

pub use types::{Event, EventHistory, EventHistoryIter, EventHistoryRangeIter, EventKey, Insert, PruneResult};
