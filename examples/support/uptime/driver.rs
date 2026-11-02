//! Nonblocking generation, delivery, monitor sealing and genuine ConTime query validation.
use super::{
    reference::Reference,
    report::{self, Report},
    Config,
};
use crate::{
    definition::Time,
    support::monitored::backend::Backend,
    uptime_example::{
        model::{Service, Status, StatusChanged},
        setup::ServiceExample,
    },
};
use contime::contime_monitor::{WorkerMonitor, WorkerReading};
use contime::contime_simulator::Random;
use crossbeam_channel::{Receiver, TryRecvError};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
    time::Duration,
};
#[path = "../../cases/service_uptime/generation.rs"]
mod generation;
type Failure = Box<dyn std::error::Error>;
struct Seal {
    time: Time,
    submitted: u64,
    final_query: bool,
    started: Duration,
}
struct Query {
    seal: Seal,
    #[allow(clippy::vec_box)] // Existing backend query transport owns boxed snapshots.
    receiver: Receiver<Vec<Box<Service>>>,
    states: Vec<Option<Service>>,
}
pub struct Driver {
    pub config: Config,
    pub now: Duration,
    pub done: bool,
    pub progressed: bool,
    pub submitted: u64,
    pub completed: u64,
    pub updates: u64,
    pub latest: Option<Arc<WorkerReading<Time>>>,
    pub baseline: Option<Arc<WorkerReading<Time>>>,
    reference: Reference,
    arrival: Reference,
    waiting: BTreeMap<(u64, u128), StatusChanged>,
    recent: VecDeque<report::Input>,
    last_seen: Vec<Time>,
    sequence: u64,
    delay_random: Random,
    next_tick: u64,
    seal: Option<Seal>,
    query: Option<Query>,
    collecting: bool,
    next_status: u64,
    next_query: u64,
    stopped: Option<(Time, Duration)>,
    target: Time,
    view: Report,
    late: u64,
}
impl Driver {
    pub fn new<B: Backend<ServiceExample>>(backend: &B, config: Config) -> Result<Self, Failure> {
        config.validate()?;
        let seeds: Vec<_> = (1..=config.services)
            .map(|id| StatusChanged { id: id as u128, service_id: id as u128, time: Time(1), status: Status::Healthy })
            .collect();
        backend.submit(seeds.clone())?;
        let mut reference = Reference::new(config.services);
        let mut arrival = Reference::new(config.services);
        for event in seeds {
            reference.insert(event.clone());
            arrival.insert(event);
        }
        Ok(Self {
            sequence: config.services as u64,
            delay_random: Random::new(42),
            submitted: config.services as u64,
            last_seen: vec![Time(1); config.services],
            config,
            reference,
            arrival,
            now: Duration::ZERO,
            done: false,
            progressed: false,
            completed: 0,
            updates: 0,
            latest: None,
            baseline: None,
            waiting: BTreeMap::new(),
            recent: VecDeque::new(),
            next_tick: 0,
            seal: None,
            query: None,
            collecting: false,
            next_status: 0,
            next_query: 0,
            stopped: None,
            target: Time(1),
            view: Report::default(),
            late: 0,
        })
    }
    pub fn in_flight(&self) -> usize {
        self.waiting.len()
    }
    pub fn late(&self) -> u64 {
        self.late
    }
    fn tick_time(&self) -> u64 {
        1_000_000_001u64.saturating_add(self.next_tick.saturating_mul(self.config.cadence_ms * 1_000_000))
    }
    fn generate(&mut self, now: u64) -> Result<(), Failure> {
        // Bounded catch-up; cap advancement below any not-yet-generated input.
        for _ in 0..crate::support::uptime_config::CATCH_UP_TICKS {
            let time = self.tick_time();
            if time > now {
                break;
            }
            for id in 1..=self.config.services {
                let phase = (self.next_tick + id as u64 - 1) % 4;
                let status = match phase {
                    0 => Status::Down,
                    2 => Status::Degraded,
                    _ => Status::Healthy,
                };
                self.sequence = self.sequence.checked_add(1).ok_or("Input identity exhausted")?;
                let radius = self.config.jitter_ms.min(self.config.latency_ms);
                // One reproducible network stream, independent of health status and service identity.
                let width = std::num::NonZeroU64::new(2 * radius + 1).expect("inclusive jitter range is nonempty");
                let delay = self.config.latency_ms - radius + self.delay_random.below(width);
                let delay = delay.max(self.config.min_delay_ms);
                let event = generation::event(id, self.sequence, Time(time), status);
                self.waiting.insert((time.saturating_add(delay * 1_000_000), event.id), event);
            }
            self.next_tick = self.next_tick.checked_add(1).ok_or("Generation clock exhausted")?;
            if self.waiting.len() > crate::support::uptime_config::MAX_PENDING_CHANGES {
                return Err("Simulated uptime network exceeded its pending capacity while processing was stalled".into());
            }
        }
        Ok(())
    }
    pub fn step<B: Backend<ServiceExample>>(
        &mut self,
        backend: &B,
        monitor: &mut WorkerMonitor<'_, Time, B>,
        elapsed: Duration,
        stop: bool,
    ) -> Result<bool, Failure> {
        if elapsed < self.now {
            return Err("Uptime clock went backwards".into());
        }
        let now = u64::try_from(elapsed.as_nanos())
            .map_err(|_| "Uptime clock exceeded its range")?
            .checked_add(1)
            .ok_or("Uptime clock exceeded its range")?;
        self.now = elapsed;
        if self.done {
            return Ok(true);
        }
        self.progressed = backend.process()?;
        backend.check_rejections()?;
        if stop || self.config.duration.is_some_and(|duration| elapsed >= duration) {
            let (at, _) = *self.stopped.get_or_insert((Time(now), elapsed));
            self.waiting.retain(|_, event| event.time <= at);
        }
        if self.stopped.is_none() {
            let mut horizon = now.saturating_add(crate::support::uptime_config::LOOK_AHEAD_MS * 1_000_000);
            if let Some(duration) = self.config.duration {
                horizon = horizon.min(duration.as_nanos() as u64 + 1);
            }
            self.generate(horizon)?;
        }
        if self.stopped.is_some_and(|(_, at)| elapsed.saturating_sub(at) > self.config.drain_timeout()) {
            return Err("Uptime drain timed out".into());
        }
        if self.seal.is_none() && self.query.is_none() && backend.can_submit() {
            // Stop flushes the simulated network in its original scheduled order; real backend work still drains normally.
            let delivery_limit = if self.stopped.is_some() { u64::MAX } else { now };
            let keys: Vec<_> = self.waiting.range(..=(delivery_limit, u128::MAX)).take(256).map(|(key, _)| *key).collect();
            if !keys.is_empty() {
                let mut events = Vec::with_capacity(keys.len());
                for key in keys {
                    let event = self.waiting.remove(&key).expect("selected pending health change");
                    let index = event.service_id as usize - 1;
                    let late = event.time < self.last_seen[index];
                    self.late += u64::from(late);
                    self.last_seen[index] = self.last_seen[index].max(event.time);
                    self.recent.push_back(report::Input {
                        id: event.id as u64,
                        service: index + 1,
                        status: status(event.status),
                        original_ms: millis(event.time.0),
                        delivery_ms: millis(key.0),
                        arrived_ms: Some(millis(now)),
                        late,
                    });
                    while self.recent.len() > report::MAX_INPUTS {
                        self.recent.pop_front();
                    }
                    let mut arrival = event.clone();
                    arrival.time = Time(now);
                    // Arrival IDs preserve handoff order when several delayed changes share one host turn.
                    arrival.id = u128::from(self.submitted + events.len() as u64 + 1);
                    self.arrival.insert(arrival);
                    self.reference.insert(event.clone());
                    events.push(event);
                }
                self.submitted += events.len() as u64;
                backend.submit(events)?;
            }
            if now >= self.next_query || (self.stopped.is_some() && self.waiting.is_empty()) {
                let mut target = Time(now);
                let oldest =
                    self.waiting.values().map(|e| e.time.0).min().into_iter().chain(self.stopped.is_none().then(|| self.tick_time())).min();
                if let Some(oldest) = oldest {
                    target = target.min(Time(oldest.saturating_add(self.config.retention_ms * 1_000_000).saturating_sub(1)));
                }
                self.target = self.target.max(target);
                backend.advance(self.target)?;
                self.seal = Some(Seal {
                    time: self.target,
                    submitted: self.submitted,
                    final_query: self.stopped.is_some() && self.waiting.is_empty(),
                    started: elapsed,
                });
                self.next_status = now;
            }
        }
        if self.seal.is_some() && !self.collecting && now >= self.next_status {
            monitor.begin(elapsed)?;
            self.collecting = true;
        }
        if self.collecting {
            if let Some(reading) = monitor.poll(elapsed)? {
                self.collecting = false;
                if self.baseline.is_none() {
                    self.baseline = Some(Arc::clone(&reading));
                }
                self.latest = Some(Arc::clone(&reading));
                if self.seal.as_ref().is_some_and(|seal| ready(&reading, seal)) {
                    let seal = self.seal.take().expect("confirmed seal");
                    self.completed = seal.submitted;
                    let (sender, receiver) = crossbeam_channel::unbounded();
                    backend.query(seal.time, self.config.services, sender)?;
                    self.query = Some(Query { seal, receiver, states: vec![None; self.config.services] });
                }
                self.next_status = now.saturating_add(2_000_000);
            }
        }
        if self.seal.as_ref().is_some_and(|s| elapsed.saturating_sub(s.started) > Duration::from_secs(10)) {
            return Err(format!(
                "Uptime worker confirmation timed out: submitted={}, target={:?}, reading={:?}",
                self.submitted, self.target, self.latest
            )
            .into());
        }
        if let Some(query) = &mut self.query {
            if elapsed.saturating_sub(query.seal.started) > Duration::from_secs(10) {
                return Err("Uptime query timed out".into());
            }
            let mut finished = false;
            loop {
                match query.receiver.try_recv() {
                    Ok(states) => {
                        for state in states {
                            let index = usize::try_from(state.service_id)
                                .ok()
                                .and_then(|id| id.checked_sub(1))
                                .filter(|id| *id < query.states.len())
                                .ok_or("Unknown queried service")?;
                            if query.states[index].is_some() {
                                return Err("Duplicate queried service".into());
                            }
                            query.states[index] = Some(*state);
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        finished = true;
                        break;
                    }
                }
            }
            if finished {
                let query = self.query.take().expect("completed query");
                let expected = self.reference.states(query.seal.time);
                let actual: Vec<_> = query.states.into_iter().collect::<Option<Vec<_>>>().ok_or("Missing queried service")?;
                for (a, e) in actual.iter().zip(&expected) {
                    if a.status != e.status
                        || a.changed_at != e.changed_at
                        || a.events != e.events
                        || a.totals_at(query.seal.time) != e.totals_at(query.seal.time)
                        || a.intervals != e.intervals
                    {
                        return Err(format!("Service {} differs from independent chronological reference", e.service_id).into());
                    }
                }
                self.updates += 1;
                self.done = query.seal.final_query;
                let now_ms = millis(query.seal.time.0);
                let start = now_ms.saturating_sub(10000);
                let arrival = self.arrival.states(query.seal.time);
                self.view = Report {
                    now_ms,
                    window_start_ms: start,
                    services: actual
                        .iter()
                        .zip(&arrival)
                        .map(|(a, b)| report::ServiceReport {
                            id: a.service_id as usize,
                            name: format!("Service {}", a.service_id),
                            reconciled: timeline(a, query.seal.time, start),
                            arrival_order: timeline(b, query.seal.time, start),
                        })
                        .collect(),
                    done: self.done,
                    ..Default::default()
                };
                let before = Time(query.seal.time.0.saturating_sub(self.config.retention_ms * 1_000_000));
                self.reference.compact(before);
                self.arrival.compact(before);
                self.next_query = now.saturating_add(1_000_000_000 / self.config.query_hz);
            }
        }
        Ok(self.done)
    }
    pub fn report(&self) -> Report {
        let mut view = self.view.clone();
        view.cadence_ms = Some(self.config.cadence_ms);
        view.clock_ms = self.now.as_millis() as u64;
        view.submitted = self.submitted;
        view.completed = self.completed;
        view.updates = self.updates;
        view.late = self.late;
        view.in_flight = self.waiting.len();
        view.done = self.done;
        // Reserve half the bounded preview for pending changes. Recent arrivals
        // retain the remainder so delivery animations still have their source.
        let pending: Vec<_> = self
            .waiting
            .iter()
            .take(report::MAX_INPUTS / 2)
            .map(|(&(due, _), event)| report::Input {
                id: event.id as u64,
                service: event.service_id as usize,
                status: status(event.status),
                original_ms: millis(event.time.0),
                delivery_ms: millis(due),
                arrived_ms: None,
                late: false,
            })
            .collect();
        let recent_capacity = report::MAX_INPUTS - pending.len();
        view.inputs = self.recent.iter().skip(self.recent.len().saturating_sub(recent_capacity)).cloned().collect();
        view.inputs.extend(pending);
        if view.services.is_empty() {
            view.inputs.clear();
        }
        view
    }
}
fn ready(reading: &WorkerReading<Time>, seal: &Seal) -> bool {
    let inserted = reading
        .workers
        .iter()
        .try_fold(0u64, |sum, w| w.counters.filter(|c| !c.saturated).and_then(|c| sum.checked_add(c.inserted_applications)));
    inserted == Some(seal.submitted)
        && reading.workers.iter().all(|w| {
            w.due_histories == 0 && w.pending_prune_passes == 0 && w.awaiting_resolution.is_none() && w.processing_target >= seal.time
        })
}
fn status(value: Status) -> report::Status {
    match value {
        Status::Healthy => report::Status::Healthy,
        Status::Degraded => report::Status::Degraded,
        Status::Down => report::Status::Down,
    }
}
// Application time reserves zero; subtract the one-nanosecond origin for presentation.
fn millis(time: u64) -> u64 {
    time.saturating_sub(1) / 1_000_000
}
fn timeline(state: &Service, time: Time, start: u64) -> report::Timeline {
    let (uptime, downtime, degraded) = state.totals_at(time);
    let mut intervals: Vec<_> = state
        .intervals
        .iter()
        .filter(|i| millis(i.end.0) >= start)
        .map(|i| report::Interval {
            start_ms: (millis(i.start.0)).max(start),
            end_ms: (millis(i.end.0)).min(millis(time.0)),
            status: status(i.status),
        })
        .collect();
    intervals.push(report::Interval {
        start_ms: (millis(state.changed_at.0)).max(start),
        end_ms: millis(time.0),
        status: status(state.status),
    });
    if intervals.len() > report::MAX_INTERVALS {
        intervals.drain(..intervals.len() - report::MAX_INTERVALS);
    }
    report::Timeline {
        status: status(state.status),
        uptime_ms: uptime / 1_000_000,
        downtime_ms: downtime / 1_000_000,
        degraded_ms: degraded / 1_000_000,
        intervals,
    }
}
#[cfg(all(test, feature = "manual"))]
#[path = "tests.rs"]
mod tests;
