//! Hash definition and simulator callbacks composed with the shared monitored host.
#[path = "../../cases/chain_hash/generation.rs"]
mod generation;
#[path = "../../cases/chain_hash/model.rs"]
mod model;
#[path = "../../cases/chain_hash/setup.rs"]
mod setup;
#[path = "../../support/mod.rs"]
mod support;
use support::{definition, monitored};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    monitored::run_configured::<setup::HashExample, _, _>(false, generation::events, monitored::Delivery::ReverseGroups).map(|_| ())
}

#[cfg(test)]
#[path = "../../cases/chain_hash/monitored_tests.rs"]
mod monitored_tests;

#[cfg(test)]
#[path = "../../cases/chain_hash/reference_tests.rs"]
mod reference_tests;
