//! Canonical ownership-generic event storage independent of ConTime orchestration.

mod history;
mod insert;
mod iteration;
mod types;

pub use types::{Event, EventHistory, EventHistoryIter, EventHistoryRangeIter, EventKey, Insert};
