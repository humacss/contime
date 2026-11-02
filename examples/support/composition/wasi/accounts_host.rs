//! WASI runs the account balance simulation; the terminal host only displays it.
use crate::{
    accounts::{Config, Driver},
    accounts_example::setup::AccountExample,
    support::monitored::{
        backend::{self, Backend},
        Options,
    },
};
use contime::contime_monitor::WorkerMonitor;
use std::{
    io::{self, Write},
    time::{Duration, Instant},
};
type Failure = Box<dyn std::error::Error>;
pub fn run(config: Config, metrics: bool, controls: Option<crate::input::Mode>) -> Result<(), String> {
    let options = Options {
        workers: config.workers,
        snapshots: 1,
        retention_ms: config.retention_ms,
        checkpoint_interval: config.checkpoint_interval,
        headless: false,
        ..Default::default()
    };
    if config.manual {
        drive(backend::Manual::<AccountExample>::new(&options).map_err(|e| e.to_string())?, config, metrics, controls)
    } else {
        #[cfg(feature = "runtime")]
        {
            drive(backend::runtime::<AccountExample>(&options).map_err(|e| e.to_string())?, config, metrics, controls)
        }
        #[cfg(not(feature = "runtime"))]
        {
            Err("Runtime requires the threaded WASI build".into())
        }
    }
}
fn drive<B: Backend<AccountExample>>(
    backend: B,
    config: Config,
    metrics: bool,
    controls: Option<crate::input::Mode>,
) -> Result<(), String> {
    let result = (|| -> Result<(), Failure> {
        let mut driver = Driver::new(config);
        let mut monitor = WorkerMonitor::attach(&backend);
        let clock = Instant::now();
        let mut next_report = Duration::ZERO;
        let mut stop = false;
        let mut input = controls.map(crate::input::Input::new).transpose()?;
        loop {
            if let Some(input) = &mut input {
                if input.read()?.contains(&b'q') {
                    stop = true;
                }
            }
            let done = driver.step(&backend, &mut monitor, clock.elapsed(), stop)?;
            if clock.elapsed() >= next_report || done {
                let report = driver.report();
                if metrics {
                    writeln!(io::stdout().lock(), "contime-workers-v2\t{}", serde_json::json!({"done":done,"accounts":report}))?;
                } else {
                    println!(
                        "{} unique={} balance={} retries={} query_ms={}",
                        if done { "DRAINED (validated)" } else { "LIVE" },
                        report.submitted,
                        report.balance,
                        report.retries,
                        report.query_ms
                    );
                }
                io::stdout().flush()?;
                next_report = clock.elapsed() + if metrics { Duration::from_millis(50) } else { Duration::from_secs(1) };
            }
            if done {
                return Ok(());
            }
            if !driver.progressed {
                std::thread::sleep(Duration::from_micros(100));
            }
        }
    })();
    let completion = result.map_err(|e| e.to_string()).and_then(|()| crate::exit::success().map_err(|e| e.to_string()));
    let shutdown = backend.shutdown().map_err(|e| e.to_string());
    completion.and(shutdown)
}
