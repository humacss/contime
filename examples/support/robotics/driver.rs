//! Caller-driven sensor delivery, worker confirmation and validated estimator queries.
use super::{reference::Reference, report, Config};
use crate::{
    definition::Time,
    robotics_example::{
        model::{Maneuver, Reading, Robot, SensorEvent},
        setup::RoboticsExample,
    },
    support::monitored::backend::Backend,
};
use contime::contime_monitor::WorkerMonitor;
use crossbeam_channel::{Receiver, TryRecvError};
use std::{
    collections::{BTreeMap, VecDeque},
    time::Duration,
};
type Failure = Box<dyn std::error::Error>;
struct Seal {
    time: Time,
    inserted: u64,
    started: Duration,
    final_query: bool,
}
pub struct Driver {
    pub config: Config,
    pub now: Duration,
    pub done: bool,
    pub progressed: bool,
    pub submitted: u64,
    next: u64,
    waiting: BTreeMap<(u64, u64), SensorEvent>,
    recent: VecDeque<report::Observation>,
    reference: Reference,
    arrival: Reference,
    seal: Option<Seal>,
    #[allow(clippy::vec_box)] // Backend query transport owns boxed snapshots.
    receiver: Option<Receiver<Vec<Box<Robot>>>>,
    collecting: bool,
    next_query: u64,
    stopped: Option<(Time, Duration)>,
    target: Time,
    view: report::Report,
}
impl Driver {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            now: Duration::ZERO,
            done: false,
            progressed: false,
            submitted: 0,
            next: 0,
            waiting: BTreeMap::new(),
            recent: VecDeque::new(),
            reference: Reference::new(),
            arrival: Reference::new(),
            seal: None,
            receiver: None,
            collecting: false,
            next_query: 0,
            stopped: None,
            target: Time(1),
            view: report::Report::default(),
        }
    }
    pub fn step<B: Backend<RoboticsExample>>(
        &mut self,
        backend: &B,
        monitor: &mut WorkerMonitor<'_, Time, B>,
        elapsed: Duration,
        stop: bool,
    ) -> Result<bool, Failure> {
        if elapsed < self.now {
            return Err("Robot clock went backwards".into());
        }
        self.now = elapsed;
        if self.done {
            return Ok(true);
        }
        let now = u64::try_from(elapsed.as_nanos())?.checked_add(1).ok_or("Robot clock exhausted")?;
        self.progressed = backend.process()?;
        backend.check_rejections()?;
        if stop || self.config.duration.is_some_and(|duration| elapsed >= duration) {
            let deadline = self.config.duration.filter(|duration| elapsed >= *duration).map(|duration| duration.as_nanos() as u64 + 1);
            let stop_time = deadline.unwrap_or(now);
            let (at, _) = *self.stopped.get_or_insert((Time(stop_time), elapsed));
            self.waiting.retain(|_, event| event.time <= at);
        }
        // Finish generating already-captured inputs even when a long host stall
        // coincides with stop. Future previews beyond that frozen horizon are omitted.
        let horizon = self.stopped.map_or(now.saturating_add(3_500_000_000), |(at, _)| at.0);
        for _ in 0..256 {
            if super::scenario::action_ns(self.next) > horizon {
                break;
            }
            if let Some((event, delay)) = super::scenario::input(self.next) {
                self.waiting.insert((event.time.0.saturating_add(delay * 1_000_000), self.next), event);
            }
            self.next = self.next.checked_add(1).ok_or("Robot sequence exhausted")?;
        }
        let ungenerated = self.stopped.is_some_and(|(at, _)| super::scenario::action_ns(self.next) <= at.0);
        if self.waiting.len() > 8192 {
            return Err("Robot network capacity exceeded".into());
        }
        if self.stopped.is_some_and(|(_, at)| elapsed.saturating_sub(at) > Duration::from_secs(15)) {
            return Err("Robot drain timed out".into());
        }
        if self.seal.is_none() && backend.can_submit() {
            let limit = if self.stopped.is_some() { u64::MAX } else { now };
            let keys: Vec<_> = self.waiting.range(..=(limit, u64::MAX)).take(64).map(|(key, _)| *key).collect();
            let mut events = Vec::new();
            for key in keys {
                let event = self.waiting.remove(&key).unwrap();
                self.submitted += 1;
                self.reference.insert(event.clone());
                let mut current = event.clone();
                if matches!(current.reading, Reading::Camera(_)) {
                    current.time = Time(now);
                    self.recent.push_back(observation(key, &event, Some(now)));
                    while self.recent.len() > 64 {
                        self.recent.pop_front();
                    }
                }
                self.arrival.insert(current);
                events.push(event);
            }
            if !events.is_empty() {
                backend.submit(events)?;
            }
            if now >= self.next_query || self.stopped.is_some() {
                let oldest = self
                    .waiting
                    .values()
                    .map(|event| event.time.0)
                    .chain((self.stopped.is_none() || ungenerated).then(|| super::scenario::action_ns(self.next)))
                    .min()
                    .unwrap_or(now);
                self.target = self.target.max(Time(now.min(oldest.saturating_add(self.config.retention_ms * 1_000_000).saturating_sub(1))));
                backend.advance(self.target)?;
                self.seal = Some(Seal {
                    time: self.target,
                    inserted: self.submitted,
                    started: elapsed,
                    final_query: self.stopped.is_some() && self.waiting.is_empty() && !ungenerated,
                });
            }
        }
        if self.seal.is_some() && self.receiver.is_none() && !self.collecting {
            monitor.begin(elapsed)?;
            self.collecting = true;
        }
        if self.collecting {
            if let Some(reading) = monitor.poll(elapsed)? {
                self.collecting = false;
                let seal = self.seal.as_ref().unwrap();
                let inserted = reading
                    .workers
                    .iter()
                    .try_fold(0u64, |sum, w| w.counters.filter(|c| !c.saturated).and_then(|c| sum.checked_add(c.inserted_applications)));
                if inserted == Some(seal.inserted)
                    && reading.workers.iter().all(|w| {
                        w.due_histories == 0
                            && w.pending_prune_passes == 0
                            && w.awaiting_resolution.is_none()
                            && w.processing_target >= seal.time
                    })
                {
                    if seal.inserted == 0 {
                        self.finish_query(Robot { robot_id: 1, ..Default::default() })?;
                    } else {
                        let (send, receive) = crossbeam_channel::unbounded();
                        backend.query(seal.time, 1, send)?;
                        self.receiver = Some(receive);
                    }
                }
            }
        }
        if let Some(receiver) = &self.receiver {
            match receiver.try_recv() {
                Ok(mut states) => {
                    if states.len() != 1 {
                        return Err("Expected one queried robotics".into());
                    }
                    self.finish_query(*states.remove(0))?;
                    self.receiver = None;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => return Err("Robot query disconnected".into()),
            }
        }
        if self.seal.as_ref().is_some_and(|seal| elapsed.saturating_sub(seal.started) > Duration::from_secs(10)) {
            return Err("Robot query confirmation timed out".into());
        }
        Ok(self.done)
    }
    fn finish_query(&mut self, state: Robot) -> Result<(), Failure> {
        let seal = self.seal.take().unwrap();
        let expected = self.reference.at(seal.time);
        if (state.pose.x - expected.pose.x).abs() > 1e-9
            || (state.pose.y - expected.pose.y).abs() > 1e-9
            || (state.pose.heading - expected.pose.heading).abs() > 1e-9
            || state.events != expected.events
            || state.observations != expected.observations
            || state.proposal != expected.proposal
            || state.route.len() != expected.route.len()
            || state.route.iter().zip(&expected.route).any(|(a, b)| (a.x - b.x).abs() > 1e-9 || (a.y - b.y).abs() > 1e-9)
            || state.trail.len() != expected.trail.len()
            || state.trail.iter().zip(&expected.trail).any(|(a, b)| {
                a.time != b.time
                    || (a.pose.x - b.pose.x).abs() > 1e-9
                    || (a.pose.y - b.pose.y).abs() > 1e-9
                    || (a.pose.heading - b.pose.heading).abs() > 1e-9
            })
        {
            return Err("Robot query disagrees with chronological reference".into());
        }
        self.view.historical = estimate(&state);
        self.view.current = estimate(&self.arrival.at(seal.time));
        self.view.query_ms = millis(seal.time.0);
        self.view.revision += 1;
        self.next_query = (self.now.as_nanos() as u64).saturating_add(1_000_000_000 / self.config.query_hz);
        let before = Time(seal.time.0.saturating_sub(self.config.retention_ms * 1_000_000));
        self.reference.compact(before);
        self.arrival.compact(before);
        self.done = seal.final_query;
        Ok(())
    }
    pub fn report(&self) -> report::Report {
        let mut report = self.view.clone();
        report.clock_ms = self.now.as_millis() as u64;
        report.submitted = self.submitted;
        report.done = self.done;
        report.truth = pose(super::scenario::truth(self.stopped.map_or(report.query_ms, |(at, _)| report.query_ms.min(millis(at.0)))));
        report.observations =
            self.recent.iter().filter(|o| o.arrived_ms.is_some_and(|at| report.clock_ms.saturating_sub(at) <= 10000)).cloned().collect();
        report.observations.extend(
            self.waiting
                .iter()
                .filter(|(_, event)| matches!(event.reading, Reading::Camera(_)))
                .take(64)
                .map(|(&key, event)| observation(key, event, None)),
        );
        report
    }
}
fn millis(time: u64) -> u64 {
    time.saturating_sub(1) / 1_000_000
}
fn observation(key: (u64, u64), event: &SensorEvent, arrived: Option<u64>) -> report::Observation {
    let Reading::Camera(measured) = event.reading else { unreachable!("camera presentation only") };
    report::Observation {
        id: event.id as u64,
        capture_ms: millis(event.time.0),
        due_ms: millis(key.0),
        arrived_ms: arrived.map(millis),
        pose: pose(measured),
    }
}
fn pose(p: crate::robotics_example::model::Pose) -> report::Pose {
    report::Pose { x: p.x, y: p.y, heading: p.heading }
}
fn estimate(state: &Robot) -> report::Estimate {
    report::Estimate {
        pose: pose(state.pose),
        route: state.route.iter().copied().map(pose).collect(),
        proposal: match state.proposal {
            Maneuver::Continue => "Continue to aisle",
            Maneuver::Backtrack => "Backtrack to aisle",
            Maneuver::Turn => "Turn into aisle",
            Maneuver::AtGoal => "At inspection goal",
        }
        .into(),
        observations: state.observations,
        trail: state.trail.iter().map(|s| report::Sample { time_ns: s.time.0, pose: pose(s.pose) }).collect(),
    }
}
#[cfg(all(test, feature = "manual"))]
#[path = "tests.rs"]
mod tests;
