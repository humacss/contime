//! Native movement composition: simulated delivery, queries and terminal presentation.
mod host;
#[path = "portable.rs"]
pub mod portable;
mod ui;
pub use crate::support::movement_config::Config;
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let Some(mut config) = Config::parse(std::env::args().skip(1))? else {
        println!("Client movement: --clients N --workers N --execution runtime|manual\n--client-spacing N --movement-speed N --input-hz N --query-hz N --average-latency-ms N --jitter-ms N --trail-ms N --fade-ms N\n--retention-ms N --checkpoint-interval N --headless --duration SECONDS --no-setup\nWASD/arrows choose direction, Space stops, q/Esc/Ctrl-C quits. Client count includes you. Spacing defaults to 4 world units and speed to 6 units/s; both accept whole numbers from 0 to 1,000,000. Latency is an average; jitter varies symmetrically around it. Retention grows automatically unless explicitly set.");
        return Ok(());
    };
    if !config.configure()? {
        return Ok(());
    }
    host::run(config)
}
