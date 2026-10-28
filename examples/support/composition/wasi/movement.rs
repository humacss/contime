//! WASI runs the same movement simulation; the terminal host only displays it.
use crate::{
    movement::{Config, Driver},
    movement_example::setup::MovementExample,
    support::{
        monitored::{
            backend::{self, Backend},
            Options,
        },
        monitored_metrics::Metrics,
        monitored_wire::Sample,
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
        snapshots: config.clients,
        retention_ms: config.retention_ms,
        checkpoint_interval: config.checkpoint_interval,
        headless: false,
        ..Default::default()
    };
    if config.manual {
        drive(backend::Manual::<MovementExample>::new(&options).map_err(|e| e.to_string())?, config, metrics, controls)
    } else {
        #[cfg(feature = "runtime")]
        {
            drive(backend::runtime::<MovementExample>(&options).map_err(|e| e.to_string())?, config, metrics, controls)
        }
        #[cfg(not(feature = "runtime"))]
        {
            Err("Runtime requires the threaded WASI build".into())
        }
    }
}
fn drive<B: Backend<MovementExample>>(
    backend: B,
    config: Config,
    metrics: bool,
    controls: Option<crate::input::Mode>,
) -> Result<(), String> {
    let result = (|| -> Result<(), Failure> {
        let mut driver = Driver::new(&backend, config)?;
        let mut monitor = WorkerMonitor::attach(&backend);
        let clock = Instant::now();
        let mut next_report = Duration::ZERO;
        let mut stop = false;
        let mut input = controls.map(crate::input::Input::new).transpose()?;
        loop {
            if let Some(input) = &mut input {
                for key in input.read()?.into_iter().filter(|b| b"wasd q".contains(b)) {
                    if key == b'q' {
                        stop = true;
                    } else {
                        driver.input(crate::movement::direction(std::str::from_utf8(&[key])?)?)?;
                    }
                }
            }
            let done = driver.step(&backend, &mut monitor, clock.elapsed(), stop)?;
            if clock.elapsed() >= next_report || done {
                let report = driver.report();
                if metrics {
                    let sample = Sample {
                        metrics: Metrics {
                            target_rate: (driver.config.clients.saturating_sub(1) as u64) * driver.config.input_hz,
                            produced: driver.submitted,
                            completed: driver.completed,
                            outstanding: driver.submitted - driver.completed,
                            elapsed: driver.now,
                            reversed: None,
                            queries: driver.updates,
                        },
                        reading: driver.latest.clone(),
                        done,
                        snapshot_count: driver.config.clients,
                        snapshots: Vec::new(),
                        movement: Some(report),
                    };
                    writeln!(io::stdout().lock(), "{}", sample.encode())?;
                } else {
                    println!(
                        "{} delivered={} reordered={} corrections={} updates={} keyboard={}",
                        if done { "DRAINED (validated)" } else { "LIVE" },
                        report.submitted,
                        report.late,
                        report.corrections,
                        report.updates,
                        report.keyboard_inputs
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standalone_wasi_movement_exits_after_validation_without_teardown() {
        use crate::support::monitored::exit_tests::{self, Observed};
        let name = concat!(module_path!(), "::standalone_wasi_movement_exits_after_validation_without_teardown");
        let name = name.split_once("::").unwrap().1;
        exit_tests::assert_fast_exit(name, |marker| {
            let mut config = Config::parse(
                ["--headless", "--execution", "manual", "--clients", "2", "--latency-ms", "0", "--jitter-ms", "0"].map(str::to_owned),
            )
            .unwrap()
            .unwrap();
            config.duration = Some(Duration::from_millis(30));
            let options = Options { snapshots: 2, ..Default::default() };
            let backend = Observed::new(backend::Manual::<MovementExample>::new(&options).unwrap(), marker);
            drive(backend, config, false, None).unwrap();
        });
    }
}
