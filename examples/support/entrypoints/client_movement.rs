//! Compose the movement lane and shared native support, without a case-aware environment.
#[path = "../../cases/client_movement/mod.rs"]
mod movement_example;
#[cfg(test)]
use movement_example::{model, setup};
#[path = "../movement/mod.rs"]
mod movement;
#[path = "../mod.rs"]
mod support;
use support::definition;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    movement::run()
}

#[cfg(test)]
#[path = "../../cases/client_movement/integration_tests.rs"]
mod integration_tests;
