//! Browser frame adapter for the shared simulator, Manual backend and monitor.
//! No blocking host loop, sleeping, or background tasks run here.
use crate::{
    counter_example::{model, setup::CounterExample},
    definition::{Example, Time},
    support::{
        monitored::{
            backend::{Backend, Manual},
            control::Controller,
            reference, Options,
        },
        Settings,
    },
};
use contime_monitor::{WorkerMonitor, WorkerReading};
use crossbeam_channel::{Receiver, TryRecvError};
use std::{sync::Arc, time::Duration};
#[path = "../../../cases/counter/monitored/generation.rs"]
mod generation;
type Generator<M> = fn(Options, u64, u64, Time) -> Vec<<M as Example>::Event>;
type Failure = Box<dyn std::error::Error>;
struct Query<M: Example> {
    #[allow(clippy::vec_box)] // Existing ConTime query transport owns boxed snapshots.
    result: Receiver<Vec<Box<M::State>>>,
    started: Duration,
    expected: Vec<M::State>,
    final_query: bool,
}
pub struct Frame<M: Example = CounterExample> {
    backend: Manual<M>,
    options: Options,
    controller: Controller,
    expected: Vec<M::State>,
    verified: Vec<M::State>,
    ids: Vec<u128>,
    applications: u64,
    last_time: Time,
    now: Duration,
    next_status: Duration,
    baseline: Option<Arc<WorkerReading<Time>>>,
    latest: Option<Arc<WorkerReading<Time>>>,
    stopping: bool,
    draining_since: Option<Duration>,
    query: Option<Query<M>>,
    generate: Generator<M>,
    reverse: bool,
    next_query: Duration,
    queries: u64,
    query_duration: Duration,
    done: bool,
}
impl Frame<CounterExample> {
    pub fn new(settings: &Settings) -> Result<Self, String> {
        Self::with_generator(settings, |o, s, c, t| generation::events(o, s, c, t).collect(), false)
    }
}
impl Frame<crate::hash_example::setup::HashExample> {
    pub fn hash(settings: &Settings) -> Result<Self, String> {
        Self::with_generator(settings, crate::hash::events, true)
    }
}
impl<M: Example> Frame<M> {
    fn with_generator(settings: &Settings, generate: Generator<M>, reverse: bool) -> Result<Self, String> {
        let options = Options::from_settings(settings);
        let backend = Manual::new(&options).map_err(|e| e.to_string())?;
        let mut controller = Controller::new(options.rate, options.targets, options.maximum);
        if let Some(limit) = options.source_limit {
            controller.limit_sources(limit);
        }
        if reverse {
            controller.reverse_groups();
        }
        Ok(Self {
            expected: vec![M::State::default(); options.snapshots],
            verified: vec![M::State::default(); options.snapshots],
            backend,
            options,
            controller,
            ids: Vec::new(),
            applications: 0,
            last_time: Time(0),
            now: Duration::ZERO,
            next_status: Duration::ZERO,
            baseline: None,
            latest: None,
            stopping: false,
            draining_since: None,
            query: None,
            generate,
            reverse,
            next_query: Duration::ZERO,
            queries: 0,
            query_duration: Duration::ZERO,
            done: false,
        })
    }
    /// One host turn: at most one existing worker process() call and one simulator batch.
    /// The outer frame loop checks its time/step budget between these complete turns.
    pub fn step(&mut self, now: Duration, stop: bool) -> Result<bool, String> {
        self.turn(now, stop).map_err(|e| e.to_string())
    }
    fn turn(&mut self, now: Duration, stop: bool) -> Result<bool, Failure> {
        if now < self.now {
            return Err("Browser active elapsed time moved backwards".into());
        }
        self.now = now;
        if self.done {
            return Ok(true);
        }
        self.stopping |= stop;
        if self.stopping {
            self.controller.stop(now);
            let began = *self.draining_since.get_or_insert(now);
            if now.saturating_sub(began) > Duration::from_secs(10) {
                return Err("Final processing confirmation timed out".into());
            }
        }
        self.backend.process()?;
        self.backend.check_rejections()?;
        if let Some(query) = &self.query {
            if now.saturating_sub(query.started) > Duration::from_secs(5) {
                return Err("Snapshot validation timed out".into());
            }
            match query.result.try_recv() {
                Ok(states) => {
                    let mut actual = vec![M::State::default(); self.options.snapshots];
                    let mut seen = vec![false; self.options.snapshots];
                    reference::receive::<M>(states, &mut actual, &mut seen)?;
                    reference::verify::<M>(&actual, &query.expected)?;
                    self.query_duration = now.saturating_sub(query.started);
                    self.verified = actual;
                    self.done = query.final_query;
                    self.query = None;
                    return Ok(self.done);
                }
                Err(TryRecvError::Empty) => return Ok(false),
                Err(TryRecvError::Disconnected) => return Err("Manual validation query closed without a result".into()),
            }
        }
        let available = if self.backend.can_submit() { self.controller.allowance(now) } else { 0 };
        if available >= self.controller.minimum_submission() && available > 0 {
            let time = Time((now.as_nanos() as u64).max(self.last_time.0 + available));
            let mut events = (self.generate)(self.options.clone(), self.controller.produced(), available, time);
            if events.len() as u64 != available {
                return Err("Simulator yielded the wrong event count".into());
            }
            let applications = reference::record::<M>(&mut events, &mut self.expected, self.options.targets, &mut self.ids)?;
            if self.reverse {
                events.reverse();
            }
            self.backend.submit(events)?;
            self.backend.advance(time)?;
            self.applications += applications;
            self.controller.submitted(available, applications, time, now)?;
            self.last_time = time;
            if self.controller.waiting() {
                self.next_status = self.next_status.min(now);
            }
        }
        let mut completed_window = false;
        if self.stopping || now >= self.next_status {
            // This source captures the one local worker synchronously. Collection
            // performs no process() call and never waits; preserve its real report.
            let mut collector = WorkerMonitor::attach(&self.backend);
            collector.begin(now)?;
            let reading = collector.poll(now)?.ok_or("Local worker status did not return immediately")?;
            // A synthetic frame clock can be unchanged across turns. Completion
            // needs a positive observation interval, so defer feedback to a later turn.
            if self.controller.window_started().is_none_or(|started| now > started) && self.controller.observe(&reading)? {
                completed_window = true;
            }
            if self.baseline.is_none() {
                self.baseline = Some(Arc::clone(&reading));
            }
            self.latest = Some(reading);
            self.next_status = now + if self.controller.waiting() { Duration::from_millis(2) } else { Duration::from_millis(50) };
        }
        if (self.stopping && self.controller.outstanding() == 0) || (self.reverse && completed_window && now >= self.next_query) {
            let (response, result) = crossbeam_channel::unbounded();
            self.backend.query(self.last_time, self.options.snapshots, response)?;
            self.queries += 1;
            self.query = Some(Query { result, started: now, expected: self.expected.clone(), final_query: self.stopping });
            self.next_query = now + Duration::from_secs_f64(1.0 / self.options.query_hz as f64);
        }
        if self.options.maximum
            && self.controller.window_started().is_some_and(|started| now.saturating_sub(started) > Duration::from_secs(10))
        {
            return Err("Production window made no confirmed completion within ten seconds".into());
        }
        Ok(false)
    }
    pub fn report(&self) -> serde_json::Value {
        let workers = crate::worker_report::workers(self.baseline.as_deref(), self.latest.as_deref());
        serde_json::json!({ "elapsed": self.now.as_secs_f64(), "target": self.controller.rate().to_string(),
            "target_rate": self.controller.rate(), "produced": self.controller.produced(), "completed": self.controller.completed(),
            "outstanding": self.controller.outstanding(), "submitted": self.controller.produced(), "applications": self.applications,
            "queued": self.backend.queued(), "queries": self.queries, "query_ms": self.query_duration.as_secs_f64()*1000.0,
            "snapshot_count": self.options.snapshots, "average_targets": self.options.average_scaled.unwrap_or(1000) as f64 /1000.0,
            "max_targets": self.options.targets, "snapshot_skew": self.options.snapshot_skew, "workers": 1,
            "workers_status": workers,
            "hashes": if self.reverse { self.expected.iter().zip(&self.verified).take(20).enumerate().map(|(index, (latest, verified))| {
                let digest = |state: &M::State| M::hash(state).map(|hash| hash.iter().map(|b| format!("{b:02x}")).collect::<String>());
                serde_json::json!({"id": index+1, "latest_count": M::count(latest), "latest_hash": digest(latest), "verified_count": M::count(verified), "verified_hash": digest(verified)})
            }).collect::<Vec<_>>() } else { Vec::new() } })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_skew_reaches_the_shared_simulator() {
        let (settings, _) = Settings::parse(
            ["--workers", "1", "--snapshots", "8", "--snapshot-skew", "100", "--max-snapshots-per-event", "2"].map(str::to_owned),
            true,
        )
        .unwrap()
        .unwrap();
        let mut counter = Frame::new(&settings).unwrap();
        for i in 1..100 {
            for _ in 0..16 {
                counter.step(Duration::from_millis(i), false).unwrap();
            }
        }
        for _ in 0..16000 {
            if counter.step(Duration::from_millis(100), true).unwrap() {
                break;
            }
        }
        assert!(counter.done, "browser stream did not drain");
        assert!(counter.verified[0].value > 0);
        assert!(counter.verified[1..].iter().all(|state| state.value == 0));
        reference::verify::<CounterExample>(&counter.verified, &counter.expected).unwrap();
    }
    #[test]
    fn monitor_completion_does_not_bypass_final_snapshot_validation() {
        let (settings, _) = Settings::parse(
            ["--workers", "1", "--snapshots", "8", "--events-per-second", "1000", "--max-pending", "64"].map(str::to_owned),
            true,
        )
        .unwrap()
        .unwrap();
        let mut counter = Frame::new(&settings).unwrap();
        for frame in 0..20 {
            for _ in 0..32 {
                counter.step(Duration::from_millis(frame * 16), false).unwrap();
            }
        }
        counter.expected.iter_mut().find(|s| s.events > 0).unwrap().value += 1;
        for _ in 0..1000 {
            match counter.step(Duration::from_millis(320), true) {
                Err(error) => {
                    assert!(error.contains("differs from generated reference"), "{error}");
                    return;
                }
                Ok(done) => assert!(!done, "completion skipped reference validation"),
            }
        }
        panic!("validation did not finish");
    }
}

#[cfg(test)]
mod hash_tests {
    use super::*;
    use crate::hash_example::setup::HashExample;
    fn settings(rate: &str, pending: &str) -> Settings {
        Settings::parse(
            [
                "--workers",
                "1",
                "--snapshots",
                "8",
                "--events-per-second",
                rate,
                "--max-pending",
                pending,
                "--retention-ms",
                "1",
                "--query-hz",
                "1000",
            ]
            .map(str::to_owned),
            true,
        )
        .unwrap()
        .unwrap()
        .0
    }
    #[test]
    fn reversed_hashes_validate_all_snapshots_with_short_retention_and_low_rate_pairs() {
        for (rate, pending) in [("1000", "64"), ("1", "2")] {
            let mut hash = Frame::<HashExample>::hash(&settings(rate, pending)).unwrap();
            for frame in 0..200 {
                for _ in 0..32 {
                    hash.step(Duration::from_millis(frame * 16), false).unwrap();
                }
            }
            assert!(hash.queries > 0);
            assert!(hash.controller.completed() > 0);
            for _ in 0..1000 {
                if hash.step(Duration::from_millis(3200), true).unwrap() {
                    break;
                }
            }
            assert!(hash.done);
            reference::verify::<HashExample>(&hash.verified, &hash.expected).unwrap();
            let rows = hash.report()["hashes"].as_array().unwrap().clone();
            assert_eq!(rows.len(), 8);
            assert!(rows.iter().all(|row| row["latest_hash"].as_str().unwrap().len() == 64));
            assert!(rows.iter().all(|row| row["latest_hash"] == row["verified_hash"]));
        }
    }
    #[test]
    fn pending_hash_query_freezes_generation_and_advancement_and_captures_reference() {
        let mut hash = Frame::<HashExample>::hash(&settings("1000", "64")).unwrap();
        for frame in 0..1000 {
            hash.step(Duration::from_millis(frame), false).unwrap();
            if hash.query.is_some() {
                break;
            }
        }
        let query = hash.query.as_ref().expect("periodic validation must start");
        let captured = query.expected.clone();
        let produced = hash.controller.produced();
        let time = hash.last_time;
        hash.expected[0].value += 100;
        for _ in 0..1000 {
            hash.step(hash.now, false).unwrap();
            assert_eq!(hash.controller.produced(), produced);
            assert_eq!(hash.last_time, time);
            if hash.query.is_none() {
                break;
            }
        }
        assert!(hash.query.is_none());
        reference::verify::<HashExample>(&hash.verified, &captured).unwrap();
    }
    #[test]
    fn final_hash_mismatch_is_fatal_even_after_monitor_completion() {
        let mut hash = Frame::<HashExample>::hash(&settings("1000", "64")).unwrap();
        hash.next_query = Duration::from_secs(60);
        for frame in 0..20 {
            for _ in 0..32 {
                hash.step(Duration::from_millis(frame * 16), false).unwrap();
            }
        }
        assert_eq!(hash.queries, 0);
        assert!(hash.controller.completed() > 0, "queries cannot provide production credit");
        hash.expected[0].hash[31] ^= 1;
        for _ in 0..1000 {
            match hash.step(Duration::from_millis(320), true) {
                Err(error) => {
                    assert!(error.contains("full digest"));
                    return;
                }
                Ok(done) => assert!(!done),
            }
        }
        panic!("final hash validation did not finish");
    }
}
