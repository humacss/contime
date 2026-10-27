//! Select a host, execution model, application definition and settings.
#[path = "../mod.rs"]
mod support;
use support::definition;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    support::launcher::run()
}
