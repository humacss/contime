//! Real-time input delivery, monitored query boundaries, and rendering-state updates.
use super::{
    portable::Driver,
    ui::{Command, View},
    Config,
};
use crate::{
    movement_example::setup::MovementExample,
    support::monitored::backend::{self, Backend},
};
use contime::contime_monitor::WorkerMonitor;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
type Failure = Box<dyn std::error::Error>;
use crate::support::native;
#[derive(Debug)]
pub struct Summary {
    pub submitted: u64,
    pub late: u64,
    pub corrections: u64,
    pub updates: u64,
    pub keyboard_inputs: u64,
}
pub fn run(config: Config) -> Result<(), Failure> {
    let options = crate::support::monitored::Options {
        workers: config.workers,
        snapshots: config.clients,
        retention_ms: config.retention_ms,
        checkpoint_interval: config.checkpoint_interval,
        headless: false,
        ..Default::default()
    };
    if config.manual {
        #[cfg(feature = "manual")]
        {
            run_backend(backend::Manual::<MovementExample>::new(&options)?, &config, true)?;
        }
        #[cfg(not(feature = "manual"))]
        {
            return Err("Enable the manual feature for Manual execution".into());
        }
    } else {
        run_backend(backend::runtime::<MovementExample>(&options)?, &config, true)?;
    }
    Ok(())
}
fn print_summary(summary: &Summary) {
    println!(
        "Validated {} delivered inputs; {} reordered arrivals; {} corrected histories; {} rendering updates",
        crate::support::number(summary.submitted),
        crate::support::number(summary.late),
        crate::support::number(summary.corrections),
        crate::support::number(summary.updates)
    );
    println!("{} keyboard inputs delivered", crate::support::number(summary.keyboard_inputs));
}
fn run_backend<B: Backend<MovementExample>>(backend: B, config: &Config, exit_on_success: bool) -> Result<Summary, Failure> {
    let result = native::run(config.headless, |terminal, cancelled| drive(&backend, config, cancelled, &mut View::new(terminal)?))
        .and_then(|summary| {
            if exit_on_success {
                print_summary(&summary);
                crate::support::exit::success()?;
            }
            Ok(summary)
        });
    let shutdown = backend.shutdown();
    let summary = result?;
    shutdown?;
    print_summary(&summary);
    Ok(summary)
}
fn drive<B: Backend<MovementExample>>(
    backend: &B,
    config: &Config,
    cancelled: &Arc<AtomicBool>,
    view: &mut View<'_>,
) -> Result<Summary, Failure> {
    let clock = Instant::now();
    let mut driver = Driver::new(backend, config.clone())?;
    let mut monitor = WorkerMonitor::attach(backend);
    let mut next_draw = Duration::ZERO;
    let mut reading_at = None;
    loop {
        let mut stop = cancelled.load(Ordering::Relaxed);
        for command in view.commands()? {
            match command {
                Command::Quit => stop = true,
                Command::Turn(direction) => driver.input(direction)?,
            }
        }
        let done = driver.step(backend, &mut monitor, clock.elapsed(), stop)?;
        if let Some(reading) = &driver.latest {
            if reading_at != Some(reading.completed_at) {
                view.workers.set_reading(Arc::clone(reading));
                reading_at = Some(reading.completed_at);
            }
        }
        if clock.elapsed() >= next_draw || done {
            view.draw(&driver.report(), driver.now)?;
            if config.headless {
                println!(
                    "Inputs delivered {} | in flight {} | reordered {} | corrected {} | render updates {}",
                    crate::support::number(driver.submitted),
                    crate::support::number(driver.in_flight()),
                    crate::support::number(driver.late()),
                    crate::support::number(driver.scene.corrections),
                    crate::support::number(driver.updates)
                );
            }
            next_draw = clock.elapsed() + if config.headless { Duration::from_secs(1) } else { Duration::from_millis(33) };
        }
        if done {
            return Ok(Summary {
                submitted: driver.submitted,
                late: driver.late(),
                corrections: driver.scene.corrections,
                updates: driver.updates,
                keyboard_inputs: driver.keyboard_inputs,
            });
        }
        if !driver.progressed {
            std::thread::sleep(Duration::from_micros(100));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standalone_movement_exits_after_validation_without_teardown() {
        use crate::support::monitored::exit_tests::{self, Observed};
        let name = concat!(module_path!(), "::standalone_movement_exits_after_validation_without_teardown");
        let name = name.split_once("::").unwrap().1;
        exit_tests::assert_fast_exit(name, |marker| {
            let mut config = Config::parse(["--headless", "--clients", "2", "--latency-ms", "0", "--jitter-ms", "0"].map(str::to_owned))
                .unwrap()
                .unwrap();
            config.duration = Some(Duration::from_millis(30));
            let options = crate::support::monitored::Options { workers: 1, snapshots: 2, ..Default::default() };
            let backend = Observed::new(backend::runtime::<MovementExample>(&options).unwrap(), marker);
            run_backend(backend, &config, true).unwrap();
        });
    }
    #[test]
    fn live_inputs_drain_and_validate_through_both_execution_adapters() {
        for manual in [false, true] {
            if manual && !cfg!(feature = "manual") {
                continue;
            }
            let mut config = Config::parse(
                ["--headless", "--clients", "4", "--input-hz", "30", "--latency-ms", "10", "--jitter-ms", "200"].map(str::to_owned),
            )
            .unwrap()
            .unwrap();
            config.duration = Some(Duration::from_millis(400));
            config.manual = manual;
            config.workers = if manual { 1 } else { 2 };
            let options = crate::support::monitored::Options {
                workers: config.workers,
                snapshots: config.clients,
                retention_ms: config.retention_ms,
                ..Default::default()
            };
            let summary = if manual {
                #[cfg(feature = "manual")]
                {
                    run_backend(backend::Manual::<MovementExample>::new(&options).unwrap(), &config, false).unwrap()
                }
                #[cfg(not(feature = "manual"))]
                {
                    unreachable!()
                }
            } else {
                run_backend(backend::runtime::<MovementExample>(&options).unwrap(), &config, false).unwrap()
            };
            assert!(summary.submitted > 4);
            assert!(summary.updates > 1);
        }
    }
}
