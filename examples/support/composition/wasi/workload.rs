//! Shared domain simulation and worker monitoring, entirely inside the guest.
use crate::{
    definition::Example,
    definition::Time,
    support::{
        monitored::{
            backend::{self, Backend},
            driver::{self, Delivery, Observer},
            Options,
        },
        monitored_metrics::Metrics,
        monitored_wire::Sample,
        Settings,
    },
    CounterExample, HashExample,
};
use contime::contime_monitor::WorkerReading;
use std::{
    io::{self, Write},
    sync::{atomic::AtomicBool, Arc},
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
}
impl<M: Example> Observer<M> for Output {
    fn interval(&self) -> Duration {
        if self.metrics_stdio {
            Duration::from_millis(50)
        } else {
            Duration::from_secs(1)
        }
    }
    fn report(
        &mut self,
        metrics: &Metrics,
        reading: Option<&Arc<WorkerReading<Time>>>,
        expected: &[M::State],
        verified: &[M::State],
        done: bool,
    ) -> Result<(), Failure> {
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
pub fn run(domain: &str, settings: Settings, runtime: bool, metrics_stdio: bool) -> Result<(), String> {
    match domain {
        "counter" => run_model::<CounterExample, _, _>(settings, runtime, metrics_stdio, &counter_generation::events, Delivery::Forward),
        "chain-hash" => run_model::<HashExample, _, _>(settings, runtime, metrics_stdio, &hash_generation::events, Delivery::ReverseGroups),
        _ => Err("Domain must be counter or chain-hash".into()),
    }
}
fn run_model<M: Example, G, It>(
    settings: Settings,
    runtime: bool,
    metrics_stdio: bool,
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
        return run_backend::<M, _, _, _>(backend, options, metrics_stdio, generate, delivery);
    }
    if runtime {
        return Err("Runtime requires a thread-capable WASI build".into());
    }
    let backend = backend::Manual::<M>::new(&options).map_err(|e| e.to_string())?;
    run_backend::<M, _, _, _>(backend, options, metrics_stdio, generate, delivery)
}
fn run_backend<M: Example, B: Backend<M>, G, It>(
    backend: B,
    options: Options,
    metrics_stdio: bool,
    generate: &G,
    delivery: Delivery,
) -> Result<(), String>
where
    G: Fn(Options, u64, u64, Time) -> It,
    It: Iterator<Item = M::Event>,
{
    let result =
        driver::drive::<M, _, _, _, _>(&backend, &options, generate, delivery, &AtomicBool::new(false), &mut Output { metrics_stdio });
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
