//! Simulator workloads in a standalone WASI host.
mod config;
use config::Options;
fn main() -> Result<(), String> {
    let Some(options) = Options::parse(std::env::args().skip(1))? else {
        println!("ConTime live WASI host\n\n--domain counter|chain-hash|client-movement (default counter)\n--execution manual|runtime (default manual)\n--snapshots N  --workers N (runtime only)\n--snapshot-skew 0..100 (0 = even, 100 = concentrated)\n--average-snapshots-per-event DECIMAL  --max-snapshots-per-event N\n--events-per-second max|N  --query-hz N  --retention-ms N\n--checkpoint-interval N  --max-pending N (numeric rates only)  --duration SECONDS\n--no-setup  --headless  --metrics-stdio  --help\n\nRuns continuously until the host is stopped (Ctrl-C).\nAn explicit --duration stops production, validates final state and exits without history teardown.");
        return Ok(());
    };
    if !options.metrics_stdio {
        println!("Execution: {}", options.execution);
    }
    if let Some(config) = options.movement {
        return contime_wasm_example::movement_host::run(config, options.metrics_stdio, options.controls);
    }
    contime_wasm_example::workload::run(&options.domain, options.settings, options.execution == "runtime", options.metrics_stdio)
}
