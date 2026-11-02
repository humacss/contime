//! Native composition for the independent service uptime workload.
#[path = "../mod.rs"]
mod support;
#[path = "../uptime/mod.rs"]
mod uptime;
#[path = "../../cases/service_uptime/mod.rs"]
mod uptime_example;
use support::{definition, exit};
#[path = "../composition/wasi/uptime_host.rs"]
mod uptime_host;
// Direct native text output has no launcher-owned control pipe.
mod input {
    pub enum Mode {}
    pub struct Input;
    impl Input {
        pub fn new(mode: Mode) -> std::io::Result<Self> {
            match mode {}
        }
        pub fn read(&mut self) -> std::io::Result<Vec<u8>> {
            Ok(Vec::new())
        }
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(config) = uptime::Config::parse(std::env::args().skip(1))? else {
        println!("Service uptime: --execution manual|runtime --services 1..16 --cadence-ms 100..60000 --latency-ms N --jitter-ms N --min-delay-ms N (default 250) --query-hz 1..60 --retention-ms N --checkpoint-interval N --workers N --headless --duration SECONDS");
        return Ok(());
    };
    if config.headless {
        uptime_host::run(config, false, None).map_err(Into::into)
    } else {
        support::launcher::uptime_graphics(config)
    }
}
