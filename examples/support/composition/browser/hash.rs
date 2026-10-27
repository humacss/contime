//! Hash case callbacks, reused by the shared nonblocking browser adapter.
use crate::{definition::Time, hash_example::model, monitored::Options};
#[path = "../../../cases/chain_hash/generation.rs"]
mod generation;
pub fn events(options: Options, start: u64, count: u64, time: Time) -> Vec<model::CounterChanged> {
    generation::events(options, start, count, time).collect()
}
