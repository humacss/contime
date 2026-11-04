//! Shared domain simulation and worker monitoring, entirely inside the guest.
use crate::{
    CounterExample, HashExample,
    definition::Example,
    definition::Time,
    support::{
        Settings,
        monitored::{
            Options,
            backend::{self, Backend},
            driver::{self, Delivery, Observer},
        },
        monitored_metrics::Metrics,
        monitored_wire::Sample,
    },
};
use contime::contime_monitor::WorkerReading;
use std::{
    io::{self, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
#[path = "../../../cases/counter/monitored/generation.rs"]
mod counter_generation;
mod model {
    pub use crate::counter_example::model::*;
}
#[path = "hash_domain.rs"]
mod hash_domain;
use hash_domain::generation as hash_generation;
type Failure = Box<dyn std::error::Error>;
struct Output {
    metrics_stdio: bool,
    controls: Option<crate::input::Input>,
    stop: Arc<AtomicBool>,
}
impl<M: Example> Observer<M> for Output {
    fn interval(&self) -> Duration {
        if self.metrics_stdio { Duration::from_millis(50) } else { Duration::from_secs(1) }
    }
    fn verification_failed(
        &mut self,
        metrics: &Metrics,
        reading: Option<&Arc<WorkerReading<Time>>>,
        expected: &[M::State],
        actual: &[M::State],
        error: &str,
    ) -> Result<(), Failure> {
        if self.metrics_stdio {
            let mut sample = Sample::new::<M>(metrics, reading, expected, actual, false);
            sample.hash_summary = M::hash(&M::State::default()).map(|_| {
                crate::support::monitored_metrics::HashSummary::compare_workers::<M>(
                    expected,
                    actual,
                    true,
                    true,
                    reading.map(|r| r.workers.len()),
                )
            });
            sample.failure = Some(error.to_owned());
            let mut output = io::stdout().lock();
            writeln!(output, "{}", sample.encode())?;
            output.flush()?;
        }
        Ok(())
    }
    fn report(
        &mut self,
        metrics: &Metrics,
        reading: Option<&Arc<WorkerReading<Time>>>,
        expected: &[M::State],
        verified: &[M::State],
        done: bool,
    ) -> Result<(), Failure> {
        if let Some(input) = &mut self.controls {
            if input.read()?.contains(&b'q') {
                self.stop.store(true, Ordering::Release);
            }
        }
        let mut output = io::stdout().lock();
        if self.metrics_stdio {
            writeln!(output, "{}", Sample::new::<M>(metrics, reading, expected, verified, done).encode())?;
        } else {
            writeln!(output, "{}\n{}", if done { "DRAINED (validated)" } else { "LIVE" }, metrics.text())?;
            for (index, state) in verified.iter().take(8).enumerate() {
                if let Some(hash) = M::hash(state) {
                    let digest = hash.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
                    writeln!(output, "snapshot={} verified-count={} verified-hash={digest}", index + 1, M::count(state))?;
                }
            }
        }
        output.flush()?;
        Ok(())
    }
}
pub fn run(
    domain: &str,
    settings: Settings,
    runtime: bool,
    metrics_stdio: bool,
    controls: Option<crate::input::Mode>,
) -> Result<(), String> {
    match domain {
        "internal-counter" => {
            run_model::<CounterExample, _, _>(settings, runtime, metrics_stdio, controls, &counter_generation::events, Delivery::Forward)
        }
        "chain-hash" => {
            run_model::<HashExample, _, _>(settings, runtime, metrics_stdio, controls, &hash_generation::events, Delivery::ReverseGroups)
        }
        _ => Err("Domain must be chain-hash".into()),
    }
}
fn run_model<M: Example, G, It>(
    settings: Settings,
    runtime: bool,
    metrics_stdio: bool,
    controls: Option<crate::input::Mode>,
    generate: &G,
    delivery: Delivery,
) -> Result<(), String>
where
    G: Fn(Options, u64, u64, Time) -> It,
    It: Iterator<Item = M::Event>,
    M::Event: Send + Sync,
    M::State: Send,
{
    let mut options = Options::from_settings(&settings);
    // Redirected output must not force a duration in a terminal-free guest.
    options.headless = false;
    #[cfg(feature = "runtime")]
    if runtime {
        let backend = backend::runtime::<M>(&options).map_err(|e| e.to_string())?;
        return run_backend::<M, _, _, _>(backend, options, metrics_stdio, controls, generate, delivery);
    }
    if runtime {
        return Err("Runtime requires a thread-capable WASI build".into());
    }
    let backend = backend::Manual::<M>::new(&options).map_err(|e| e.to_string())?;
    run_backend::<M, _, _, _>(backend, options, metrics_stdio, controls, generate, delivery)
}
fn run_backend<M: Example, B: Backend<M>, G, It>(
    backend: B,
    options: Options,
    metrics_stdio: bool,
    controls: Option<crate::input::Mode>,
    generate: &G,
    delivery: Delivery,
) -> Result<(), String>
where
    G: Fn(Options, u64, u64, Time) -> It,
    It: Iterator<Item = M::Event>,
{
    let stop = Arc::new(AtomicBool::new(false));
    let controls = controls.map(crate::input::Input::new).transpose().map_err(|e| e.to_string())?;
    let mut output = Output { metrics_stdio, controls, stop: stop.clone() };
    let result = driver::drive::<M, _, _, _, _>(&backend, &options, generate, delivery, &stop, &mut output);
    // A final record certifies confirmed completion and reference validation.
    let completion = result.map(|_| ()).map_err(|e| e.to_string()).and_then(|()| crate::exit::success().map_err(|e| e.to_string()));
    let shutdown = backend.shutdown().map_err(|e| e.to_string());
    completion.and(shutdown)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Capture {
        frames: Vec<Sample>,
    }
    impl<M: Example> Observer<M> for Capture {
        fn verification_failed(
            &mut self,
            metrics: &Metrics,
            reading: Option<&Arc<WorkerReading<Time>>>,
            expected: &[M::State],
            actual: &[M::State],
            error: &str,
        ) -> Result<(), Failure> {
            let mut frame = Sample::new::<M>(metrics, reading, expected, actual, false);
            frame.hash_summary = Some(crate::support::monitored_metrics::HashSummary::compare_workers::<M>(
                expected,
                actual,
                true,
                true,
                reading.map(|r| r.workers.len()),
            ));
            frame.failure = Some(error.into());
            self.frames.push(Sample::decode(&frame.encode())?);
            Ok(())
        }
        fn report(
            &mut self,
            metrics: &Metrics,
            reading: Option<&Arc<WorkerReading<Time>>>,
            expected: &[M::State],
            verified: &[M::State],
            done: bool,
        ) -> Result<(), Failure> {
            let frame = Sample::new::<M>(metrics, reading, expected, verified, done);
            self.frames.push(Sample::decode(&frame.encode())?);
            Ok(())
        }
    }
    struct WrongHash;
    impl Example for WrongHash {
        type Event = <HashExample as Example>::Event;
        type State = <HashExample as Example>::State;
        const TITLE: &'static str = "Incorrect reference";
        const VALUE_LABEL: &'static str = "Value";
        fn event(id: u128, time: Time, targets: Vec<u128>, value: i64) -> Self::Event {
            HashExample::event(id, time, targets, value)
        }
        fn id(state: &Self::State) -> u128 {
            HashExample::id(state)
        }
        fn count(state: &Self::State) -> u64 {
            HashExample::count(state)
        }
        fn value(state: &Self::State) -> i64 {
            HashExample::value(state)
        }
        fn hash(state: &Self::State) -> Option<[u8; 32]> {
            HashExample::hash(state)
        }
        fn record(state: &mut Self::State, event: &Self::Event) {
            HashExample::record(state, event);
            state.hash[31] ^= 1;
        }
    }
    #[test]
    fn mismatch_stops_the_workload_and_emits_a_failed_summary() {
        let options = Options {
            snapshots: 1,
            targets: 1,
            rate: 1000,
            maximum: false,
            duration: Some(Duration::from_millis(50)),
            ..Options::default()
        };
        let backend = backend::Manual::<WrongHash>::new(&options).unwrap();
        let mut capture = Capture::default();
        let result = driver::drive::<WrongHash, _, _, _, _>(
            &backend,
            &options,
            &hash_generation::events,
            Delivery::ReverseGroups,
            &AtomicBool::new(false),
            &mut capture,
        );
        assert!(result.unwrap_err().to_string().contains("full digest"));
        let report = capture.frames.last().unwrap();
        assert!(report.failure.as_ref().unwrap().contains("full digest"));
        assert!(report.hash_summary.as_ref().unwrap().failed > 0);
        assert!(!report.done);
        backend.shutdown().unwrap();
    }
    #[test]
    fn guest_counter_emits_validated_worker_frames_in_both_load_modes() {
        for maximum in [false, true] {
            let options = Options {
                workers: 1,
                snapshots: 8,
                targets: 3,
                average_scaled: Some(1500),
                snapshot_skew: 90,
                maximum,
                rate: 2000,
                retention_ms: 20,
                query_hz: 1,
                duration: Some(Duration::from_millis(250)),
                ..Options::default()
            };
            let backend = backend::Manual::<CounterExample>::new(&options).unwrap();
            let mut capture = Capture::default();
            let summary = driver::drive::<CounterExample, _, _, _, _>(
                &backend,
                &options,
                &counter_generation::events,
                Delivery::Forward,
                &AtomicBool::new(false),
                &mut capture,
            )
            .unwrap();
            assert!(summary.events > 0);
            assert_eq!(summary.events, summary.completed);
            let last = capture.frames.last().unwrap();
            assert!(last.done);
            assert_eq!(last.metrics.outstanding, 0);
            assert_eq!(last.metrics.produced, summary.events);
            assert_eq!(last.reading.as_ref().unwrap().workers[0].counters.unwrap().inserted_applications, summary.applications);
            backend.shutdown().unwrap();
        }
    }
    #[test]
    fn guest_hash_validates_periodically_and_finally_after_reversed_arrivals() {
        for maximum in [false, true] {
            let options = Options {
                snapshots: 8,
                targets: 3,
                average_scaled: Some(1500),
                snapshot_skew: 90,
                maximum,
                rate: 4000,
                retention_ms: 1,
                query_hz: 100,
                duration: Some(Duration::from_millis(250)),
                ..Options::default()
            };
            let backend = backend::Manual::<HashExample>::new(&options).unwrap();
            let mut capture = Capture::default();
            let summary = driver::drive::<HashExample, _, _, _, _>(
                &backend,
                &options,
                &hash_generation::events,
                Delivery::ReverseGroups,
                &AtomicBool::new(false),
                &mut capture,
            )
            .unwrap();
            assert!(summary.events > 0);
            assert_eq!(summary.events, summary.completed);
            assert!(summary.reversed > 0);
            assert!(summary.queries > 1, "periodic and final validation must both run");
            let last = capture.frames.last().unwrap();
            assert!(last.done);
            assert_eq!(last.metrics.reversed, Some(summary.reversed));
            assert_eq!(last.metrics.queries, summary.queries);
            assert_eq!(last.reading.as_ref().unwrap().workers[0].counters.unwrap().inserted_applications, summary.applications);
            backend.shutdown().unwrap();
        }
    }
}
