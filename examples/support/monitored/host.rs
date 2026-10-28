//! Native process lifecycle and terminal presentation around the shared loop.
use super::{
    backend::{self, Backend},
    driver::{self, Delivery, Observer, Summary},
    format, input, ui, Failure, Options,
};
use crate::definition::{Example, Time};
use crate::support::native;
use crate::support::ui::workers::WorkerView;
use contime_monitor::WorkerReading;
use std::{sync::Arc, time::Duration};
/// Standalone startup form and simulator callbacks; successful runs exit the process.
/// Reusable hosts and tests use the returning `run_*_with_options` helpers.
pub fn run_configured<M, G, It>(manual_default: bool, generate: G, delivery: Delivery) -> Result<(), Failure>
where
    M: Example,
    M::Event: Send + Sync,
    M::State: Send,
    G: Fn(Options, u64, u64, Time) -> It,
    It: Iterator<Item = M::Event>,
{
    use super::super::{execution::Kind, Settings};
    let execution = Kind::select(std::env::args().skip(1), manual_default)?;
    let settings = match execution {
        Kind::Manual => Settings::from_env_manual(M::TITLE)?,
        Kind::Runtime => Settings::from_env(M::TITLE)?,
    };
    let Some(settings) = settings else { return Ok(()) };
    let options = Options::from_settings(&settings);
    match execution {
        Kind::Manual => {
            #[cfg(feature = "manual")]
            {
                let contime = backend::Manual::<M>::new(&options)?;
                run_backend::<M, G, It, _>(contime, options, generate, delivery, true).map(|_| ())
            }
            #[cfg(not(feature = "manual"))]
            {
                Err("Manual execution requires the manual feature".into())
            }
        }
        Kind::Runtime => {
            #[cfg(feature = "runtime")]
            {
                options.validate()?;
                let contime = backend::runtime::<M>(&options)?;
                run_backend::<M, G, It, _>(contime, options, generate, delivery, true).map(|_| ())
            }
            #[cfg(not(feature = "runtime"))]
            {
                Err("Runtime execution requires the runtime feature".into())
            }
        }
    }
}
#[cfg(feature = "runtime")]
pub fn run_with_options<M, G, It>(options: Options, generate: G, delivery: Delivery) -> Result<Summary, Failure>
where
    M: Example,
    M::Event: Send + Sync,
    M::State: Send,
    G: Fn(Options, u64, u64, Time) -> It,
    It: Iterator<Item = M::Event>,
{
    options.validate()?;
    let contime = backend::runtime::<M>(&options)?;
    run_backend::<M, G, It, _>(contime, options, generate, delivery, false)
}
#[cfg(feature = "manual")]
pub fn run_manual_with_options<M, G, It>(options: Options, generate: G, delivery: Delivery) -> Result<Summary, Failure>
where
    M: Example,
    G: Fn(Options, u64, u64, Time) -> It,
    It: Iterator<Item = M::Event>,
{
    let contime = backend::Manual::<M>::new(&options)?;
    run_backend::<M, G, It, _>(contime, options, generate, delivery, false)
}
fn run_backend<M, G, It, B>(
    contime: B,
    options: Options,
    generate: G,
    delivery: Delivery,
    exit_on_success: bool,
) -> Result<Summary, Failure>
where
    M: Example,
    B: Backend<M>,
    G: Fn(Options, u64, u64, Time) -> It,
    It: Iterator<Item = M::Event>,
{
    let result = native::run(options.headless, |terminal, cancelled| {
        driver::drive::<M, G, It, B, _>(&contime, &options, &generate, delivery, cancelled, &mut NativeView::new(terminal, &options)?)
    })
    .and_then(|summary| {
        if exit_on_success {
            print_summary(&summary, delivery);
            // The host has restored the terminal and the driver validated final state.
            crate::support::exit::success()?;
        }
        Ok(summary)
    });
    // Reusable runners and errors still release resources and inspect shutdown.
    let shutdown = contime.shutdown();
    let summary = result?;
    shutdown?;
    print_summary(&summary, delivery);
    Ok(summary)
}
fn print_summary(summary: &Summary, delivery: Delivery) {
    println!(
        "Validated {} events / {} snapshot applications; {} worker readings",
        format::number(summary.events),
        format::number(summary.applications),
        format::number(summary.collections)
    );
    println!(
        "Target {} events/s; run average {} completed events/s; {} unconfirmed events",
        format::number(summary.target_rate),
        format::decimal(summary.completed as f64 / summary.elapsed.as_secs_f64().max(f64::EPSILON), 0),
        format::number(summary.events - summary.completed)
    );
    if delivery == Delivery::ReverseGroups {
        println!(
            "{} reversed arrivals; {} full-state validation queries",
            format::number(summary.reversed),
            format::number(summary.queries)
        );
    }
}

struct NativeView<'a> {
    terminal: Option<&'a mut ratatui::DefaultTerminal>,
    input: Option<input::Input>,
    view: WorkerView<Time>,
    snapshots: ui::SnapshotView,
    count: usize,
    headless: bool,
    last: Option<Arc<WorkerReading<Time>>>,
}
impl<'a> NativeView<'a> {
    fn new(terminal: Option<&'a mut ratatui::DefaultTerminal>, options: &Options) -> Result<Self, Failure> {
        let input = terminal.as_ref().map(|_| input::Input::start()).transpose()?;
        Ok(Self {
            terminal,
            input,
            view: WorkerView::default(),
            snapshots: ui::SnapshotView::default(),
            count: options.snapshots,
            headless: options.headless,
            last: None,
        })
    }
}
impl<M: Example> Observer<M> for NativeView<'_> {
    fn should_stop(&mut self) -> Result<bool, Failure> {
        match &self.input {
            Some(input) => Ok(ui::quit(input, &mut self.view, &mut self.snapshots, self.count, M::hash(&M::State::default()).is_some())?),
            None => Ok(false),
        }
    }
    fn interval(&self) -> Duration {
        if self.headless {
            Duration::from_secs(1)
        } else {
            Duration::from_millis(50)
        }
    }
    fn report(
        &mut self,
        metrics: &ui::Metrics,
        reading: Option<&Arc<WorkerReading<Time>>>,
        expected: &[M::State],
        verified: &[M::State],
        done: bool,
    ) -> Result<(), Failure> {
        if let Some(reading) = reading {
            if self.last.as_ref().is_none_or(|last| !Arc::ptr_eq(last, reading)) {
                self.view.set_reading(Arc::clone(reading));
                self.last = Some(Arc::clone(reading));
            }
        }
        if !done {
            if let Some(terminal) = self.terminal.as_deref_mut() {
                ui::draw::<M>(terminal, &mut self.view, metrics.elapsed, metrics, expected, verified, &mut self.snapshots)?;
            } else if self.headless {
                println!("{}", metrics.text());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
