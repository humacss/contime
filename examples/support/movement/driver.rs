//! One complete caller-driven movement turn. Hosts supply elapsed time and input.
use super::{
    delivery::Delivery,
    queries::{poll_cycle, ready, Cycle, Seal},
    reference::Reference,
    render_state::Scene,
    Config,
};
use crate::{
    definition::Time,
    movement_example::{
        model::{Direction, Motion, MovementInput},
        setup::MovementExample,
    },
    support::monitored::backend::Backend,
};
use contime::contime_monitor::{WorkerMonitor, WorkerReading};
use std::{sync::Arc, time::Duration};
#[path = "../../cases/client_movement/generation.rs"]
mod generation;
type Failure = Box<dyn std::error::Error>;
pub struct Driver {
    pub config: Config,
    sequence: u64,
    motion: Motion,
    reference: Reference,
    delivery: Delivery,
    pub scene: Scene,
    pub latest: Option<Arc<WorkerReading<Time>>>,
    pub baseline: Option<Arc<WorkerReading<Time>>>,
    collecting: bool,
    next_status: u64,
    seal: Option<Seal>,
    cycle: Option<Cycle>,
    stop: Option<Time>,
    stopped_at: Option<Duration>,
    next_inputs: u64,
    next_query: u64,
    last_target: Time,
    pub updates: u64,
    pub submitted: u64,
    pub keyboard_inputs: u64,
    pub completed: u64,
    pub now: Duration,
    pub done: bool,
    pub progressed: bool,
    pending_input: Vec<Direction>,
}
impl Driver {
    pub fn new<B: Backend<MovementExample>>(backend: &B, config: Config) -> Result<Self, Failure> {
        config.validate()?;
        let motion = Motion { client_spacing: config.client_spacing, movement_speed: config.movement_speed };
        let seeds: Vec<_> = (1..=config.clients)
            .map(|id| MovementInput { id: id as u128, client_id: id as u128, time: Time(1), direction: Direction::Stop, motion })
            .collect();
        backend.submit(seeds.clone())?;
        let mut reference = Reference::new(config.clients, motion);
        for input in seeds {
            reference.insert(input);
        }
        Ok(Self {
            sequence: config.clients as u64,
            submitted: config.clients as u64,
            config,
            motion,
            reference,
            delivery: Delivery::new(),
            scene: Scene::default(),
            latest: None,
            baseline: None,
            collecting: false,
            next_status: 0,
            seal: None,
            cycle: None,
            stop: None,
            stopped_at: None,
            next_inputs: 0,
            next_query: 0,
            last_target: Time(1),
            updates: 0,
            keyboard_inputs: 0,
            completed: 0,
            now: Duration::ZERO,
            done: false,
            progressed: false,
            pending_input: Vec::new(),
        })
    }
    pub fn input(&mut self, direction: Direction) -> Result<(), String> {
        if self.pending_input.len() >= 64 {
            return Err("Too many keyboard inputs in one host turn".into());
        }
        if self.stop.is_none() && !self.done {
            self.pending_input.push(direction);
        }
        Ok(())
    }
    pub fn in_flight(&self) -> usize {
        self.delivery.len()
    }
    pub fn late(&self) -> u64 {
        self.delivery.late
    }
    /// At most one backend process() call; never sleeps or waits for a response.
    pub fn step<B: Backend<MovementExample>>(
        &mut self,
        backend: &B,
        monitor: &mut WorkerMonitor<'_, Time, B>,
        elapsed: Duration,
        stop: bool,
    ) -> Result<bool, Failure> {
        if elapsed < self.now {
            return Err("Movement clock went backwards".into());
        }
        self.now = elapsed;
        if self.done {
            return Ok(true);
        }
        self.progressed = backend.process()?;
        backend.check_rejections()?;
        let now = u64::try_from(elapsed.as_nanos())
            .map_err(|_| "Simulation clock exceeded its range")?
            .checked_add(1)
            .ok_or("Simulation clock exceeded its range")?;
        if stop || self.config.duration.is_some_and(|d| elapsed >= d) {
            self.stop.get_or_insert(Time(now));
            self.stopped_at.get_or_insert(elapsed);
        }
        for direction in self.pending_input.drain(..) {
            self.sequence += 1;
            self.delivery.schedule(
                MovementInput { id: self.sequence.into(), client_id: 1, time: Time(now), direction, motion: self.motion },
                now,
                self.config.average_latency_ms,
                self.config.jitter_ms,
            );
        }
        if self.stop.is_none() && now >= self.next_inputs {
            for input in generation::events(self.config.clients, self.sequence, Time(now), self.motion) {
                self.delivery.schedule(input, now, self.config.average_latency_ms, self.config.jitter_ms);
                self.sequence += 1;
            }
            self.next_inputs = now + 1_000_000_000 / self.config.input_hz;
        }
        if self.stopped_at.is_some_and(|at| elapsed.saturating_sub(at) > self.config.drain_timeout()) {
            return Err("Final movement drain timed out".into());
        }
        if self.delivery.len() > 1_000_000 {
            return Err("Simulated network reached one million queued inputs; lower client count/input frequency".into());
        }
        if self.seal.is_none() && self.cycle.is_none() && backend.can_submit() {
            let inputs = self.delivery.take_ready(now);
            if !inputs.is_empty() {
                backend.submit(inputs.clone())?;
                self.submitted += inputs.len() as u64;
                for input in inputs {
                    self.reference.insert(input);
                }
            }
            if now >= self.next_query || (self.stop.is_some() && self.delivery.len() == 0) {
                let active = self.stop.unwrap_or(Time(now));
                self.last_target = self.last_target.max(self.delivery.query_target(active, self.config.retention_ms));
                backend.advance(self.last_target)?;
                self.seal = Some(Seal {
                    time: self.last_target,
                    submitted: self.submitted,
                    final_query: self.stop.is_some() && self.delivery.len() == 0,
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
                if self.seal.as_ref().is_some_and(|s| ready(&reading, s)) {
                    let sealed = self.seal.take().expect("checked seal");
                    self.completed = sealed.submitted;
                    self.cycle = Some(Cycle::new(sealed, self.config.trail_ms));
                }
                self.next_status = now + 2_000_000;
            }
        }
        if self.seal.as_ref().is_some_and(|s| elapsed.saturating_sub(s.started) > Duration::from_secs(5)) {
            return Err("Worker confirmation for rendering timed out".into());
        }
        if let Some(queries) = &mut self.cycle {
            if poll_cycle(queries, backend, &self.reference, self.config.clients, elapsed)? {
                let queries = self.cycle.take().expect("completed rendering cycle");
                self.scene.publish(queries.rendering, now);
                self.updates += 1;
                self.reference.compact(Time(queries.seal.time.0.saturating_sub(self.config.retention_ms * 1_000_000)));
                if queries.seal.final_query {
                    self.keyboard_inputs = self.reference.states(self.last_target)[0].events.saturating_sub(1);
                    self.done = true;
                }
                self.next_query = now + 1_000_000_000 / self.config.query_hz;
            }
        }
        self.scene.expire(now, self.config.fade_ms * 1_000_000);
        Ok(self.done)
    }
}
