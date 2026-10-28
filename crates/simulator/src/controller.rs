use crate::contime_monitor::{Monitor, Totals};
use crate::contime_simulator::Error;
use std::time::Duration;

/// Units in which admission is paced. Monitoring always retains both totals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorkUnit {
    /// Source events, irrespective of their fanout.
    Events,
    /// Target snapshot applications.
    #[default]
    Applications,
}
impl WorkUnit {
    fn count(self, totals: Totals) -> u64 {
        match self {
            Self::Events => totals.events,
            Self::Applications => totals.applications,
        }
    }
}
/// Maximum-mode policy. All durations use the host's monotonic elapsed clock.
#[derive(Clone, Copy, Debug)]
pub struct MaximumConfig {
    /// Initial units per second; independent of query frequency.
    pub initial_rate: u64,
    /// Expected interval between query requests.
    pub observation_interval: Duration,
    /// Absolute query response budget, after which admission pauses.
    pub query_budget: Duration,
    /// Largest whole event's work, which must fit in a pacing burst.
    pub minimum_work: u64,
    /// Hard safety ceiling for unconfirmed work.
    pub maximum_outstanding: u64,
    /// Hard upper bound for the adaptive rate.
    pub maximum_rate: u64,
    /// Admission units; use applications when event fanout varies.
    pub unit: WorkUnit,
}
impl Default for MaximumConfig {
    fn default() -> Self {
        Self {
            initial_rate: 200_000,
            observation_interval: Duration::from_millis(50),
            query_budget: Duration::from_millis(50),
            minimum_work: 1,
            maximum_outstanding: 10_000_000,
            maximum_rate: 1_000_000_000,
            unit: WorkUnit::Applications,
        }
    }
}
/// Paced Maximum admission driven only by validated query-prefix observations.
///
/// This controller neither submits nor processes work. It never sleeps. A late
/// query pauses admission immediately; completing it cannot subsequently grow
/// the rate. A prompt fresh query allows at most 10% growth when constrained.
pub struct Maximum {
    config: MaximumConfig,
    rate: u64,
    credit: u128,
    at: Duration,
    constrained: bool,
    completed: Option<u64>,
    adjusted: Option<u64>,
    confirmed: u64,
    stale: bool,
    paused: bool,
}
impl Maximum {
    /// Construct a controller, rejecting zero or inconsistent bounds.
    pub fn new(config: MaximumConfig) -> Result<Self, Error> {
        if config.initial_rate == 0
            || config.maximum_rate < config.initial_rate
            || config.minimum_work == 0
            || config.maximum_outstanding < config.minimum_work
            || config.observation_interval.is_zero()
            || config.query_budget.is_zero()
        {
            return Err(Error::InvalidConfig);
        }
        let mut result = Self {
            config,
            rate: config.initial_rate,
            credit: 0,
            at: Duration::ZERO,
            constrained: false,
            completed: None,
            adjusted: None,
            confirmed: 0,
            stale: false,
            paused: false,
        };
        result.credit = u128::from(result.burst()) * 1_000_000_000;
        Ok(result)
    }
    fn burst(&self) -> u64 {
        self.rate.div_ceil(1000).max(self.config.minimum_work).min(self.config.maximum_outstanding)
    }
    fn decrease(&mut self) {
        self.rate = self.rate.saturating_sub((self.rate / 5).max(1)).max(1);
        self.credit = self.credit.min(u128::from(self.burst()) * 1_000_000_000);
        self.constrained = false;
    }
    /// Last successful admission refresh on the host clock.
    pub fn observed_at(&self) -> Duration {
        self.at
    }
    /// Current paced units per second.
    pub fn rate(&self) -> u64 {
        self.rate
    }
    /// Current bounded unconfirmed-work window, sized for one observation period.
    pub fn limit(&self) -> u64 {
        let nanos = self.config.observation_interval.saturating_add(self.config.query_budget).as_nanos();
        let window = u128::from(self.rate).saturating_mul(nanos).div_ceil(1_000_000_000).min(u128::from(u64::MAX)) as u64;
        window.saturating_add(self.burst()).clamp(self.config.minimum_work, self.config.maximum_outstanding)
    }
    /// Whether missing or overdue feedback currently prevents admission.
    pub fn paused(&self) -> bool {
        self.paused
    }
    /// Record that pacing or backlog capacity constrained the producer.
    pub fn constrained(&mut self) {
        self.constrained = true;
    }
    /// Refresh feedback and return whole units currently admissible.
    ///
    /// Call before constructing an event. Submit the whole event only when its
    /// cost fits, then call [`Self::consume`] and record its totals in the monitor.
    pub fn allowance(&mut self, now: Duration, monitor: &Monitor) -> Result<u64, Error> {
        if now < self.at || now < monitor.observed_at() {
            return Err(Error::ClockReversed);
        }
        self.credit = (self.credit.saturating_add((now - self.at).as_nanos().saturating_mul(u128::from(self.rate))))
            .min(u128::from(self.burst()) * 1_000_000_000);
        self.at = now;
        if let Some(observation) = monitor.last_query() {
            if self.completed != Some(observation.query.id) {
                let confirmed = self.config.unit.count(monitor.confirmed());
                if self.adjusted != Some(observation.query.id) {
                    let older = self.config.unit.count(monitor.snapshot(now).map_err(Error::Monitor)?.older_outstanding);
                    if observation.duration > self.config.query_budget || older > 0 {
                        self.decrease();
                        self.adjusted = Some(observation.query.id);
                    } else if confirmed > self.confirmed
                        && self.constrained
                        && now.saturating_sub(observation.completed_at)
                            <= self.config.observation_interval.saturating_add(self.config.query_budget)
                    {
                        self.rate = self.rate.saturating_add((self.rate / 10).max(1)).min(self.config.maximum_rate);
                    }
                }
                self.completed = Some(observation.query.id);
                self.confirmed = confirmed;
                self.constrained = false;
            }
        }
        self.paused = false;
        if let Some(query) = monitor.active_query() {
            if now.saturating_sub(query.started) > self.config.query_budget {
                self.paused = true;
                if self.adjusted != Some(query.id) {
                    self.decrease();
                    self.adjusted = Some(query.id);
                }
            }
            self.stale = false;
        } else {
            let last = monitor.last_query().map_or(Duration::ZERO, |q| q.query.started);
            let outstanding = self.config.unit.count(monitor.submitted()).saturating_sub(self.config.unit.count(monitor.confirmed()));
            if outstanding > 0 && now.saturating_sub(last) > self.config.observation_interval.saturating_add(self.config.query_budget) {
                self.paused = true;
                if !self.stale {
                    if monitor.last_query().map(|q| q.query.id) != self.adjusted || self.adjusted.is_none() {
                        self.decrease();
                    }
                    self.stale = true;
                }
            } else {
                self.stale = false;
            }
        }
        if self.paused {
            self.credit = 0;
            return Ok(0);
        }
        let outstanding = self.config.unit.count(monitor.submitted()).saturating_sub(self.config.unit.count(monitor.confirmed()));
        Ok(((self.credit / 1_000_000_000) as u64).min(self.limit().saturating_sub(outstanding)))
    }
    /// Debit work after successful submission. An overdraw leaves credit intact.
    pub fn consume(&mut self, work: u64) -> Result<(), Error> {
        if u128::from(work) * 1_000_000_000 > self.credit {
            return Err(Error::InsufficientCredit);
        }
        self.credit -= u128::from(work) * 1_000_000_000;
        Ok(())
    }
}
