//! Compose the counter with the shared simulator/monitor host, defaulting to Manual.
#[path = "../../cases/counter/monitored/generation.rs"]
mod generation;
#[path = "../../cases/counter/model.rs"]
mod model;
#[path = "../../cases/counter/setup.rs"]
mod setup;
#[path = "../../support/mod.rs"]
mod support;
use definition::Time;
use monitored::Options;
use support::{definition, monitored};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    monitored::run_configured::<setup::CounterExample, _, _>(true, generation::events, monitored::Delivery::Forward)
}

#[cfg(all(test, feature = "manual"))]
#[path = "../../cases/counter/monitored/manual_tests.rs"]
mod manual_monitored_tests;
