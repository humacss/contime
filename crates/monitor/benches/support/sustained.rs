//! Persistent Runtime with a fixed query/sampling schedule and no adaptive controller.
use super::pipeline::{Change, Counter, Mode, Time};
use contime_monitor::{Monitor, Snapshot as Metrics};
use contime_runtime::{checkpoints::CheckpointConfig, ConTime, ConTimeConfig, Placement};
use crossbeam_channel::{Receiver, Sender};
use std::time::Duration;
const BLOCK: usize = 256;
// Preserve the origin's phase while skipping missed ticks; never add a full
// interval after a slow query and silently drift the entire schedule.
pub(crate) fn next_tick(at: Duration, interval: Duration) -> Duration {
    let nanos = u64::try_from(interval.as_nanos()).unwrap();
    assert!(nanos > 0);
    let remaining = nanos - (at.as_nanos() % u128::from(nanos)) as u64;
    at.saturating_add(Duration::from_nanos(remaining))
}
#[derive(Clone, Copy)]
pub struct Config {
    pub workers: usize,
    pub snapshots: usize,
    pub fanout: usize,
    pub query_interval: Duration,
    pub sample_interval: Duration,
    pub maximum_pending: u64,
    pub retention: u64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            workers: 1,
            snapshots: 128,
            fanout: 1,
            query_interval: Duration::from_millis(50),
            sample_interval: Duration::from_millis(50),
            maximum_pending: 262_144,
            retention: 262_144,
        }
    }
}
#[derive(Clone, Copy, Default, Debug)]
pub struct Stats {
    pub events: u64,
    pub applications: u64,
    pub queried_events: u64,
    pub periodic_queries: u64,
    pub capacity_queries: u64,
    pub final_queries: u64,
    pub sample_ticks: u64,
    pub maximum_pending: u64,
    pub history_len: usize,
    pub pruned_horizon: u64,
}
#[derive(Clone, Copy)]
enum QueryReason {
    Periodic,
    Capacity,
    Final,
}
pub struct Harness {
    runtime: ConTime<Change, Counter, ()>,
    config: Config,
    monitor: Option<Monitor>,
    last_view: Option<Metrics>,
    expected: Vec<Counter>,
    responses: Sender<contime_runtime::RejectionMessage<contime_runtime::RejectionReason>>,
    inserted: Receiver<contime_runtime::RejectionMessage<contime_runtime::RejectionReason>>,
    horizons: Receiver<Time>,
    next_query: Duration,
    next_sample: Duration,
    stats: Stats,
}
impl Harness {
    pub fn new(config: Config, mode: Mode) -> Self {
        assert!(config.workers > 0 && config.snapshots > 0 && config.fanout > 0 && config.fanout <= config.snapshots);
        assert!(config.maximum_pending >= BLOCK as u64 && config.maximum_pending.is_multiple_of(BLOCK as u64));
        assert!(!config.query_interval.is_zero() && !config.sample_interval.is_zero());
        assert!(matches!(mode, Mode::Off | Mode::PerEvent));
        let runtime = ConTime::start(
            ConTimeConfig {
                router_count: 1,
                worker_count: config.workers,
                placement: Placement::default(),
                pruning_interval: Duration::from_millis(10),
                history_retention: Time(config.retention),
                checkpoints: CheckpointConfig { interval: 128 },
            },
            (),
        )
        .unwrap();
        let horizons = runtime.subscribe_pruned_horizon().unwrap();
        let mut monitor = (mode == Mode::PerEvent).then(Monitor::default);
        if let Some(m) = &mut monitor {
            m.sample(Duration::ZERO).unwrap();
        }
        let (responses, inserted) = crossbeam_channel::unbounded();
        Self {
            runtime,
            config,
            monitor,
            last_view: None,
            expected: (1..=config.snapshots).map(|id| Counter { id: id as u128, ..Counter::default() }).collect(),
            responses,
            inserted,
            horizons,
            next_query: config.query_interval,
            next_sample: config.sample_interval,
            stats: Stats::default(),
        }
    }
    /// Continuously submit unique events; query boundaries never cut a reversed block.
    /// Generation, reference updates, periodic validation and monitoring are all timed.
    pub fn run_events(&mut self, count: u64, clock: &mut impl FnMut() -> Duration) {
        let through = self.stats.events.checked_add(count).unwrap();
        while self.stats.events < through {
            let first = self.stats.events + 1;
            let end = (first + BLOCK as u64 - 1).min(through);
            // Independent canonical reference is updated in ascending order.
            for sequence in first..=end {
                for offset in 0..self.config.fanout {
                    let index = (sequence as usize - 1 + offset) % self.config.snapshots;
                    self.expected[index].count += 1;
                    self.expected[index].hash = self.expected[index].hash.rotate_left(7) ^ sequence.wrapping_mul(0x9e3779b97f4a7c15);
                }
            }
            for sequence in (first..=end).rev() {
                let event = Change { sequence, snapshots: self.config.snapshots, fanout: self.config.fanout };
                self.runtime.send(std::iter::once(event), self.responses.clone()).unwrap();
                if let Some(m) = &mut self.monitor {
                    m.record_submission(contime_monitor::Totals { events: 1, applications: self.config.fanout as u64 }).unwrap();
                }
                self.stats.events += 1;
                self.stats.applications += self.config.fanout as u64;
            }
            let pending = self.stats.events - self.stats.queried_events;
            self.stats.maximum_pending = self.stats.maximum_pending.max(pending);
            let at = clock();
            self.sample_if_due(at);
            if at >= self.next_query {
                let completed = self.query(at, QueryReason::Periodic, clock);
                self.next_query = next_tick(completed, self.config.query_interval);
            } else if pending >= self.config.maximum_pending {
                self.query(at, QueryReason::Capacity, clock);
            }
        }
    }
    fn sample_if_due(&mut self, at: Duration) {
        if at >= self.next_sample {
            self.stats.sample_ticks += 1;
            if let Some(m) = &mut self.monitor {
                self.last_view = Some(m.sample(at).unwrap());
            }
            self.next_sample = next_tick(at, self.config.sample_interval);
        }
    }
    fn query(&mut self, at: Duration, reason: QueryReason, clock: &mut impl FnMut() -> Duration) -> Duration {
        let token = self.monitor.as_mut().map(|m| m.start_query(at).unwrap());
        let (responses, inserted) = crossbeam_channel::unbounded();
        drop(std::mem::replace(&mut self.responses, responses));
        let previous = std::mem::replace(&mut self.inserted, inserted);
        assert!(
            previous.recv_timeout(Duration::from_secs(10)).is_err_and(|error| error == crossbeam_channel::RecvTimeoutError::Disconnected),
            "insertion rejected or timed out"
        );
        self.runtime.advance_to(Time(self.stats.events)).unwrap();
        let states = self.runtime.query_at(Time(self.stats.events), (1..=self.config.snapshots).map(|id| id as u128)).unwrap();
        let mut seen = vec![false; self.config.snapshots];
        for state in states {
            let index = usize::try_from(state.id).unwrap().checked_sub(1).unwrap();
            assert!(!seen[index], "duplicate snapshot");
            seen[index] = true;
            assert_eq!(state.count, self.expected[index].count);
            assert_eq!(state.hash, self.expected[index].hash, "canonical hash mismatch");
        }
        for (index, expected) in self.expected.iter().enumerate() {
            assert_eq!(seen[index], expected.count > 0);
        }
        let completed = clock();
        if let Some(m) = &mut self.monitor {
            m.complete_query(token.unwrap(), completed).unwrap();
        }
        self.stats.queried_events = self.stats.events;
        match reason {
            QueryReason::Periodic => self.stats.periodic_queries += 1,
            QueryReason::Capacity => self.stats.capacity_queries += 1,
            QueryReason::Final => self.stats.final_queries += 1,
        }
        self.sample_if_due(completed);
        completed
    }
    /// Called at a measured callback boundary, never at each periodic query.
    /// Includes any residual query and live replay drain in the timing result.
    pub fn settle(&mut self, clock: &mut impl FnMut() -> Duration) {
        if self.stats.queried_events < self.stats.events {
            self.query(clock(), QueryReason::Final, clock);
        }
        self.runtime.wait_until_idle(Duration::from_secs(10)).unwrap();
        let at = clock();
        if let Some(m) = &mut self.monitor {
            self.last_view = Some(m.sample(at).unwrap());
        }
    }
    pub fn stats(&mut self) -> Stats {
        for horizon in self.horizons.try_iter() {
            self.stats.pruned_horizon = horizon.0;
        }
        Stats { history_len: self.monitor.as_ref().map_or(0, |m| m.history().len()), ..self.stats }
    }
    pub fn view(&self) -> Option<Metrics> {
        self.last_view
    }
    pub fn shutdown(self) {
        self.runtime.shutdown();
    }
}
