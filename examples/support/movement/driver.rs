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
#[path = "routes.rs"]
mod routes;
#[path = "showcase.rs"]
mod showcase;
type Failure = Box<dyn std::error::Error>;
pub struct Driver {
    pub config: Config,
    sequence: u64,
    motion: Motion,
    pub(super) reference: Reference,
    pub(super) delivery: Delivery,
    pub scene: Scene,
    pub(super) arrival_state: super::arrival_state::ArrivalState,
    pub latest: Option<Arc<WorkerReading<Time>>>,
    pub baseline: Option<Arc<WorkerReading<Time>>>,
    collecting: bool,
    next_status: u64,
    seal: Option<Seal>,
    cycle: Option<Cycle>,
    stop: Option<Time>,
    stopped_at: Option<Duration>,
    next_inputs: u64,
    pub(super) showcase: bool,
    pub(super) intended_paths: Vec<crate::support::movement_report::IntendedPath>,
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
    pub(super) arrivals: std::collections::VecDeque<(MovementInput, u64)>,
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
        let arrival_state = super::arrival_state::ArrivalState::new(config.clients, motion);
        let intended_paths = routes::paths(&config, false);
        Ok(Self {
            intended_paths,
            sequence: config.clients as u64,
            submitted: config.clients as u64,
            config,
            motion,
            reference,
            delivery: Delivery::new(),
            scene: Scene::default(),
            arrival_state,
            latest: None,
            baseline: None,
            collecting: false,
            next_status: 0,
            seal: None,
            cycle: None,
            stop: None,
            stopped_at: None,
            next_inputs: 0,
            showcase: false,
            next_query: 0,
            last_target: Time(1),
            updates: 0,
            keyboard_inputs: 0,
            completed: 0,
            now: Duration::ZERO,
            done: false,
            progressed: false,
            pending_input: Vec::new(),
            arrivals: std::collections::VecDeque::new(),
        })
    }
    /// Opt into the fixed recording choreography before processing begins.
    pub fn use_showcase(&mut self) -> Result<(), String> {
        if self.config.clients != 5
            || self.motion.client_spacing != 18
            || self.motion.movement_speed != 8
            || self.config.input_hz != 1
            || self.config.max_latency_ms() < 1700
        {
            return Err(
                "Showcase requires five clients, spacing 18, movement speed 8, input frequency 1 and a maximum delay of at least 1,700 ms"
                    .into(),
            );
        }
        self.intended_paths = routes::paths(&self.config, true);
        self.showcase = true;
        Ok(())
    }
    pub fn input(&mut self, direction: Direction) -> Result<(), String> {
        if self.showcase {
            return Ok(());
        }
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
        let displayed = self.scene.current.samples.last().map_or(now, |sample| sample.time.0);
        self.arrival_state.expire(displayed, self.config.trail_ms);
        if stop || self.config.duration.is_some_and(|d| elapsed >= d) {
            self.stop.get_or_insert(Time(now));
            self.stopped_at.get_or_insert(elapsed);
            self.delivery.discard_after(self.stop.expect("stop was set"));
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
        if self.stop.is_none() {
            // Preview the visible future, while keeping generation bounded per host turn.
            // Scripted recordings retain their original generation schedule.
            let future = crate::support::movement_report::timeline_window(self.config.average_latency_ms, self.config.jitter_ms).1;
            let horizon = now.saturating_add(future as u64);
            for _ in 0..1024 {
                if self.showcase {
                    let (input, delay_ms) = showcase::input(self.next_inputs, self.motion);
                    if input.time.0 > now {
                        break;
                    }
                    self.sequence = input.id as u64;
                    let at = input.time.0;
                    self.delivery.schedule(input, at, delay_ms, 0);
                    self.next_inputs += 1;
                    continue;
                }
                let Some((client, time)) = generation::scheduled(self.config.clients, self.config.input_hz, self.next_inputs) else {
                    break;
                };
                if time.0 > horizon {
                    break;
                }
                self.sequence += 1;
                let input =
                    generation::event(client, self.sequence, time, self.motion, self.next_inputs / (self.config.clients - 1) as u64);
                self.delivery.schedule(input, time.0, self.config.average_latency_ms, self.config.jitter_ms);
                self.next_inputs += 1;
            }
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
                    self.arrival_state.deliver(&input, now);
                    self.arrivals.push_back((input.clone(), now));
                    while self.arrivals.len() > 128 {
                        self.arrivals.pop_front();
                    }
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
        self.arrivals.retain(|(input, _)| input.time.0 >= self.last_target.0.saturating_sub(self.config.trail_ms * 1_000_000));
        self.scene.expire(now, self.config.fade_ms * 1_000_000);
        Ok(self.done)
    }
}

#[cfg(all(test, feature = "manual"))]
mod comparison_tests {
    use super::*;
    use crate::support::monitored::{backend, Options};

    #[test]
    fn bots_generate_on_shared_ticks_but_keep_independent_network_delays() {
        let config = Config::parse(["--execution", "manual"].map(str::to_owned)).unwrap().unwrap();
        let backend = backend::Manual::<MovementExample>::new(&Options { snapshots: 5, ..Default::default() }).unwrap();
        let mut driver = Driver::new(&backend, config).unwrap();
        let mut monitor = WorkerMonitor::attach(&backend);
        let initialized = driver.submitted;
        driver.step(&backend, &mut monitor, Duration::ZERO, false).unwrap();
        let preview = driver.report();
        assert!(!preview.network.is_empty(), "preview upcoming bot inputs at startup");
        assert!(preview.network.iter().all(|input| input.action_ns > preview.now_ns && input.arrived_ns.is_none()));
        assert_eq!(preview.submitted, initialized, "previewing an input must not submit it");
        preview.validate().unwrap();
        for ms in 1..=1000 {
            driver.step(&backend, &mut monitor, Duration::from_millis(ms), false).unwrap();
        }
        let report = driver.report();
        assert_eq!(report.bot_hz, Some(1));
        let tick: Vec<_> = report.network.iter().filter(|input| input.action_ns == 1_000_000_001).collect();
        assert_eq!(tick.len(), 4);
        assert!(report.network.iter().any(|input| input.action_ns > report.now_ns));
        let deliveries: std::collections::BTreeSet<_> = tick.iter().map(|input| input.delivery_ns).collect();
        assert!(deliveries.len() > 1, "delivery jitter must remain independent across bots");
        for ms in 1001..8000 {
            if driver.step(&backend, &mut monitor, Duration::from_millis(ms), true).unwrap() {
                break;
            }
        }
        assert!(driver.done);
        assert_eq!(driver.submitted, initialized + 4, "stop must discard actions that have not occurred");
        drop(monitor);
        backend.shutdown().unwrap();
    }
    #[test]
    fn late_out_of_order_input_repairs_only_the_reconciled_view_on_the_same_query_clock() {
        let config = Config::parse(["--headless", "--clients", "1"].map(str::to_owned)).unwrap().unwrap();
        let backend = backend::Manual::<MovementExample>::new(&Options { snapshots: 1, ..Default::default() }).unwrap();
        let mut driver = Driver::new(&backend, config).unwrap();
        let mut monitor = WorkerMonitor::attach(&backend);
        let input = |id: u128, action_ms: u64, direction| MovementInput {
            id,
            client_id: 1,
            time: Time(action_ms * 1_000_000 + 1),
            direction,
            motion: driver.motion,
        };
        // The old North input is delivered after the newer Stop. Both views
        // receive that exact delivery order through the real driver pipeline.
        driver.delivery.schedule(input(2, 0, Direction::East), 1, 0, 0);
        driver.delivery.schedule(input(3, 1000, Direction::North), 1_000_000_001, 1000, 0);
        driver.delivery.schedule(input(4, 1500, Direction::Stop), 1_500_000_001, 0, 0);
        for ms in 0..5000 {
            driver.step(&backend, &mut monitor, Duration::from_millis(ms), ms >= 3000).unwrap();
            if driver.done {
                break;
            }
        }
        assert!(driver.done, "the real worker must drain and finish its validated queries");
        let report = driver.report();
        report.validate().unwrap();
        let query_time = driver.scene.current.samples.last().unwrap().time.0;
        assert_eq!(query_time, 3_000_000_001);
        assert_eq!(report.positions[0].position, [6.0, 3.0]);
        assert_eq!(report.positions[0].direction, [0.0, 0.0]);
        assert!(report.corrections > 0);

        let delivery = |id| report.network.iter().find(|event| event.id == id).unwrap();
        let east = delivery(2).arrived_ns.unwrap();
        let north = delivery(3).arrived_ns.unwrap();
        let stop = delivery(4).arrived_ns.unwrap();
        assert!(east < stop && stop < north);
        assert!(delivery(3).reordered);
        // This side integrates the actual handoff times, without repairing its
        // earlier East movement or ignoring the late North command after Stop.
        let expected = [6.0 * (stop - east) as f64 / 1e9, 6.0 * (query_time - north) as f64 / 1e9];
        for (actual, expected) in report.unreconciled_positions[0].position.into_iter().zip(expected) {
            assert!((actual - expected).abs() < 1e-9, "both views must use the completed query time");
        }
        assert_eq!(report.unreconciled_positions[0].direction, [0.0, 1.0]);
        assert_eq!(report.arrivals.iter().map(|input| input.id).collect::<Vec<_>>(), vec![2, 4, 3]);
        assert_eq!(report.unreconciled_arrivals.iter().map(|input| input.id).collect::<Vec<_>>(), vec![2, 4, 3]);
        drop(monitor);
        backend.shutdown().unwrap();
    }

    #[test]
    fn zero_delay_views_share_the_query_clock_and_remain_frozen_after_stop() {
        let config =
            Config::parse(["--headless", "--clients", "1", "--query-hz", "1", "--latency-ms", "0", "--jitter-ms", "0"].map(str::to_owned))
                .unwrap()
                .unwrap();
        let backend = backend::Manual::<MovementExample>::new(&Options { snapshots: 1, ..Default::default() }).unwrap();
        let mut driver = Driver::new(&backend, config).unwrap();
        let mut monitor = WorkerMonitor::attach(&backend);
        driver.input(Direction::East).unwrap();
        let mut observations = 0;
        for ms in 0..4000 {
            driver.step(&backend, &mut monitor, Duration::from_millis(ms), ms >= 2000).unwrap();
            let report = driver.report();
            if !report.positions.is_empty() {
                assert_eq!(report.positions[0].position, report.unreconciled_positions[0].position);
                observations += 1;
            }
            if driver.done {
                let final_position = report.positions[0].position;
                driver.step(&backend, &mut monitor, Duration::from_secs(10), true).unwrap();
                let final_report = driver.report();
                assert_eq!(final_report.positions[0].position, final_position);
                assert_eq!(final_report.unreconciled_positions[0].position, final_position);
                break;
            }
        }
        assert!(driver.done && observations > 100);
        drop(monitor);
        backend.shutdown().unwrap();
    }
}
