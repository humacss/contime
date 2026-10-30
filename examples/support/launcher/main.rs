//! Select a host, execution model, application definition and settings.
// The bootstrap builds without monitoring; the full dashboard uses monitor types.
#[cfg(not(all(feature = "runtime", feature = "monitor", feature = "macros")))]
mod bootstrap;
#[cfg(all(feature = "runtime", feature = "monitor", feature = "macros"))]
#[path = "../mod.rs"]
mod support;
#[cfg(all(feature = "runtime", feature = "monitor", feature = "macros"))]
use support::definition;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(all(feature = "runtime", feature = "monitor", feature = "macros"))]
    {
        support::launcher::run()
    }
    #[cfg(not(all(feature = "runtime", feature = "monitor", feature = "macros")))]
    {
        bootstrap::run()
    }
}
