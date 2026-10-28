use crate::contime_monitor::{Monitor, Observation, Totals};
use crate::contime_simulator::{Error, Maximum, MaximumConfig, WorkUnit};
use std::{num::NonZeroU64, time::Duration};

/// Domain-specific generation and independent query validation.
///
/// Implementations may use any event, snapshot, timestamp and expectation types.
/// `generate` updates the independent expected model, never ConTime's state.
pub trait Workload {
    /// Event submitted to the host's execution backend.
    type Event;
    /// Snapshot returned by a query.
    type Snapshot;
    /// Owned reference for exactly the prefix captured at query start.
    type Expectation;
    /// Domain validation failure.
    type Error;
    /// Predict the next whole event's number of target applications without generating it.
    fn next_applications(&self) -> NonZeroU64;
    /// Generate one event and update the independent reference model.
    fn generate(&mut self, sequence: u64, at: Duration) -> Self::Event;
    /// Capture the expected result for the current submitted prefix.
    fn expectation(&self) -> Self::Expectation;
    /// Validate every queried snapshot against the captured reference.
    fn validate(&self, expected: &Self::Expectation, actual: &[Self::Snapshot]) -> Result<(), Self::Error>;
}
/// A query prefix and its independent domain expectation.
pub struct Query<E> {
    token: crate::contime_monitor::Query,
    expected: E,
}
/// Simulation or host submission failure.
#[derive(Debug)]
pub enum ProduceError<E> {
    /// Invalid simulation operation.
    Simulation(Error),
    /// Host rejected submission; the simulation cannot continue.
    Submission(E),
}
/// A query failed domain validation or monitor bookkeeping.
#[derive(Debug)]
pub enum QueryError<E> {
    /// Domain validation failed. No progress was confirmed.
    Validation(E),
    /// The query token or clock was invalid.
    Monitor(crate::contime_monitor::Error),
    /// The simulation previously failed a submission.
    Poisoned,
}
/// A small, caller-driven Maximum-mode simulator.
///
/// The host starts Runtime or Manual, calls `produce`, issues queries, validates
/// results through `complete_query`, and owns retention and shutdown. No worker,
/// transport, timer, background thread or UI is created here.
pub struct Simulator<W: Workload> {
    workload: W,
    monitor: Monitor,
    maximum: Maximum,
    minimum_work: u64,
    unit: WorkUnit,
    poisoned: bool,
}
impl<W: Workload> Simulator<W> {
    /// Start a simulation at elapsed time zero.
    pub fn new(workload: W, config: MaximumConfig) -> Result<Self, Error> {
        let maximum = Maximum::new(config)?;
        let mut monitor = Monitor::default();
        monitor.sample(Duration::ZERO).expect("zero initializes an empty clock");
        Ok(Self { workload, monitor, maximum, minimum_work: config.minimum_work, unit: config.unit, poisoned: false })
    }
    /// Generate and immediately submit at most one whole event if capacity exists.
    ///
    /// Set `minimum_work` to the largest possible event cost. The closure must
    /// either accept the whole event or fail. Submission failure poisons this
    /// simulation because generation may already have changed its reference.
    pub fn produce<E>(&mut self, at: Duration, submit: impl FnOnce(W::Event) -> Result<(), E>) -> Result<bool, ProduceError<E>> {
        let fail = ProduceError::Simulation;
        if self.poisoned {
            return Err(fail(Error::Poisoned));
        }
        let applications = self.workload.next_applications().get();
        let cost = if self.unit == WorkUnit::Events { 1 } else { applications };
        if cost > self.minimum_work {
            return Err(fail(Error::EventTooLarge));
        }
        let available = self.maximum.allowance(at, &self.monitor).map_err(fail)?;
        if available < cost {
            self.maximum.constrained();
            return Ok(false);
        }
        let previous = self.monitor.submitted();
        let sequence = previous.events.checked_add(1).ok_or_else(|| fail(Error::CounterOverflow))?;
        previous.applications.checked_add(applications).ok_or_else(|| fail(Error::CounterOverflow))?;
        let event = self.workload.generate(sequence, at);
        if let Err(error) = submit(event) {
            self.poisoned = true;
            return Err(ProduceError::Submission(error));
        }
        self.maximum.consume(cost).map_err(fail)?;
        self.monitor.record_submission(Totals { events: 1, applications }).expect("totals prechecked");
        Ok(true)
    }
    /// Capture the submitted prefix immediately before issuing its host query.
    pub fn start_query(&mut self, at: Duration) -> Result<Query<W::Expectation>, Error> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        if at < self.maximum.observed_at() {
            return Err(Error::ClockReversed);
        }
        let token = self.monitor.start_query(at).map_err(Error::Monitor)?;
        Ok(Query { token, expected: self.workload.expectation() })
    }
    /// Validate a complete query result before confirming its captured prefix.
    ///
    /// Validation failure leaves monitor progress unchanged and should terminate
    /// the run. Confirmation describes query coverage, not background replay.
    pub fn complete_query(
        &mut self,
        query: Query<W::Expectation>,
        actual: &[W::Snapshot],
        at: Duration,
    ) -> Result<Observation, QueryError<W::Error>> {
        if self.poisoned {
            return Err(QueryError::Poisoned);
        }
        if at < self.maximum.observed_at() {
            return Err(QueryError::Monitor(crate::contime_monitor::Error::ClockReversed));
        }
        self.workload.validate(&query.expected, actual).map_err(QueryError::Validation)?;
        self.monitor.complete_query(query.token, at).map_err(QueryError::Monitor)
    }
    /// Record a bounded metrics sample for rates and UI charts.
    pub fn sample(&mut self, at: Duration) -> Result<crate::contime_monitor::Snapshot, crate::contime_monitor::Error> {
        if at < self.maximum.observed_at() {
            return Err(crate::contime_monitor::Error::ClockReversed);
        }
        self.monitor.sample(at)
    }
    /// Query-confirmed metrics. Sampling is explicit and independent of rendering.
    pub fn monitor(&self) -> &Monitor {
        &self.monitor
    }
    /// Current controller rate, window and pause state.
    pub fn controller(&self) -> &Maximum {
        &self.maximum
    }
    /// Read-only access to the independent domain model.
    pub fn workload(&self) -> &W {
        &self.workload
    }
}

impl<E: std::fmt::Display> std::fmt::Display for ProduceError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Simulation(error) => write!(f, "simulation: {error}"),
            Self::Submission(error) => write!(f, "submission: {error}"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for ProduceError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Simulation(error) => Some(error),
            Self::Submission(error) => Some(error),
        }
    }
}
impl<E: std::fmt::Display> std::fmt::Display for QueryError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation(error) => write!(f, "query validation: {error}"),
            Self::Monitor(error) => write!(f, "query observation: {error}"),
            Self::Poisoned => write!(f, "a previous submission failed"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for QueryError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Validation(error) => Some(error),
            Self::Monitor(error) => Some(error),
            Self::Poisoned => None,
        }
    }
}
