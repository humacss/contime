use crate::contime_monitor::{Error, Observation, Query, Rates, Snapshot, Totals};
use std::{collections::VecDeque, num::NonZeroUsize, time::Duration};

#[derive(Debug, Clone, Copy)]
struct Boundary {
    at: Duration,
    submitted: Totals,
    confirmed: Totals,
}

#[derive(Debug, Clone, Copy)]
struct Cohort {
    at: Duration,
    through: Totals,
}

/// Caller-driven monitor with bounded samples and submission cohorts.
/// No execution engine, clock reads, background work, or per-event instrumentation.
#[derive(Debug)]
pub struct Monitor {
    capacity: usize,
    submitted: Totals,
    confirmed: Totals,
    next_id: u64,
    active: Option<Query>,
    last_query: Option<Observation>,
    clock: Duration,
    first: Option<Boundary>,
    previous: Option<Boundary>,
    cohorts: VecDeque<Cohort>,
    history: VecDeque<Snapshot>,
}

impl Default for Monitor {
    fn default() -> Self {
        Self::with_capacity(120)
    }
}

impl Monitor {
    /// Keep at most `capacity` historical snapshots and compacted work cohorts.
    pub fn new(capacity: NonZeroUsize) -> Self {
        Self::with_capacity(capacity.get())
    }

    fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity,
            submitted: Totals::default(),
            confirmed: Totals::default(),
            next_id: 0,
            active: None,
            last_query: None,
            clock: Duration::ZERO,
            first: None,
            previous: None,
            cohorts: VecDeque::new(),
            history: VecDeque::new(),
        }
    }

    /// Add a host batch of admitted work, atomically checking both counters.
    /// This allocates no cohorts and does not read or advance time.
    pub fn record_submission(&mut self, delta: Totals) -> Result<(), Error> {
        let submitted = self.submitted.checked_add(delta).ok_or(Error::CounterOverflow)?;
        self.submitted = submitted;
        Ok(())
    }

    /// Capture the current submission prefix before issuing a host query.
    pub fn start_query(&mut self, at: Duration) -> Result<Query, Error> {
        self.start_query_through(at, self.submitted)
    }

    /// Capture an explicitly admitted prefix when newer work is already recorded.
    /// Both counters must be between the confirmed prefix and recorded submissions.
    /// The host must establish that its ordered query covers this entire prefix.
    pub fn start_query_through(&mut self, at: Duration, submitted: Totals) -> Result<Query, Error> {
        self.check_clock(at)?;
        if self.active.is_some() {
            return Err(Error::QueryAlreadyActive);
        }
        if submitted.events < self.confirmed.events
            || submitted.applications < self.confirmed.applications
            || submitted.events > self.submitted.events
            || submitted.applications > self.submitted.applications
        {
            return Err(Error::InvalidPrefix);
        }
        let id = self.next_id.checked_add(1).ok_or(Error::QueryIdOverflow)?;
        let query = Query { id, submitted, started: at };
        self.next_id = id;
        self.active = Some(query);
        self.clock = at;
        Ok(query)
    }

    /// Confirm only the captured prefix after the host has validated query coverage.
    /// Query reconstruction is not evidence of completed live replay.
    pub fn complete_query(&mut self, query: Query, at: Duration) -> Result<Observation, Error> {
        if self.active != Some(query) {
            return Err(Error::InvalidQuery);
        }
        self.check_clock(at)?;
        let observation = Observation { query, completed_at: at, duration: at - query.started };
        self.confirmed = query.submitted;
        self.active = None;
        self.last_query = Some(observation);
        self.clock = at;
        self.remove_confirmed_cohorts();
        Ok(observation)
    }

    /// Record a sample and advance the baseline used for recent rates and older work.
    pub fn sample(&mut self, at: Duration) -> Result<Snapshot, Error> {
        self.check_clock(at)?;
        let snapshot = self.snapshot(at)?;
        if snapshot.outstanding != Totals::default() {
            let through = self.cohorts.back().map(|x| x.through).unwrap_or(self.confirmed);
            if self.submitted != through {
                self.cohorts.push_back(Cohort { at: self.previous.map(|x| x.at).unwrap_or(at), through: self.submitted });
            }
        }
        while self.cohorts.len() > self.capacity {
            // Merge the oldest intervals, keeping their earliest timestamp.
            if let Some(first) = self.cohorts.pop_front() {
                if let Some(second) = self.cohorts.front_mut() {
                    second.at = first.at;
                }
            }
        }
        let boundary = Boundary { at, submitted: self.submitted, confirmed: self.confirmed };
        self.first.get_or_insert(boundary);
        self.previous = Some(boundary);
        self.clock = at;
        if self.history.len() == self.capacity {
            self.history.pop_front();
        }
        self.history.push_back(snapshot);
        Ok(snapshot)
    }

    /// Read current state without changing history, cohorts, baselines, or the clock.
    /// Recent rates are relative to the last sample, not the last read.
    pub fn snapshot(&self, at: Duration) -> Result<Snapshot, Error> {
        self.check_clock(at)?;
        let outstanding = self.submitted.since(self.confirmed);
        let rates = |baseline: Option<Boundary>| match baseline {
            Some(b) => {
                (Rates::between(self.submitted.since(b.submitted), at - b.at), Rates::between(self.confirmed.since(b.confirmed), at - b.at))
            }
            None => (Rates::default(), Rates::default()),
        };
        let recent = rates(self.previous);
        let average = rates(self.first);
        let oldest_at = self
            .cohorts
            .front()
            .map(|x| x.at)
            .or_else(|| (outstanding != Totals::default()).then(|| self.previous.map(|b| b.at).unwrap_or(at)));
        Ok(Snapshot {
            at,
            submitted: self.submitted,
            confirmed: self.confirmed,
            outstanding,
            older_outstanding: self.previous.map(|x| x.submitted.since(self.confirmed)).unwrap_or_default(),
            oldest_outstanding_age: oldest_at.map(|start| at - start),
            active_query: self.active,
            last_query: self.last_query,
            recent_submitted: recent.0,
            recent_confirmed: recent.1,
            average_submitted: average.0,
            average_confirmed: average.1,
        })
    }

    /// Retained samples in chronological order, oldest first.
    pub fn history(&self) -> impl ExactSizeIterator<Item = &Snapshot> + DoubleEndedIterator {
        self.history.iter()
    }

    /// Time of the most recent successful timed mutation, initially zero.
    /// Read-only snapshots and submissions do not advance this watermark.
    /// Rejected operations leave it unchanged.
    pub fn observed_at(&self) -> Duration {
        self.clock
    }

    /// Cumulative recorded submissions.
    pub fn submitted(&self) -> Totals {
        self.submitted
    }
    /// Latest query-confirmed prefix.
    pub fn confirmed(&self) -> Totals {
        self.confirmed
    }
    /// Current query token.
    pub fn active_query(&self) -> Option<Query> {
        self.active
    }
    /// Latest completed query and latency.
    pub fn last_query(&self) -> Option<Observation> {
        self.last_query
    }

    fn check_clock(&self, at: Duration) -> Result<(), Error> {
        if at < self.clock {
            Err(Error::ClockReversed)
        } else {
            Ok(())
        }
    }

    fn remove_confirmed_cohorts(&mut self) {
        while self.cohorts.front().is_some_and(|c| c.through.since(self.confirmed) == Totals::default()) {
            self.cohorts.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_sampling_bounds_both_retained_collections() {
        let mut monitor = Monitor::new(NonZeroUsize::MIN);
        monitor.sample(Duration::ZERO).unwrap();
        for second in 1..=1000 {
            monitor.record_submission(Totals { events: 1, applications: 3 }).unwrap();
            monitor.sample(Duration::from_secs(second)).unwrap();
            assert!(monitor.cohorts.len() <= 1);
            assert!(monitor.history.len() <= 1);
        }
        let query = monitor.start_query(Duration::from_secs(1000)).unwrap();
        monitor.complete_query(query, Duration::from_secs(1000)).unwrap();
        assert!(monitor.cohorts.is_empty());
        assert_eq!(monitor.snapshot(Duration::from_secs(1001)).unwrap().oldest_outstanding_age, None);
    }

    #[test]
    fn exhausted_query_identity_rejects_without_advancing_clock() {
        let mut monitor = Monitor { next_id: u64::MAX, ..Monitor::default() };
        assert_eq!(monitor.start_query(Duration::from_secs(10)), Err(Error::QueryIdOverflow));
        assert_eq!(monitor.active_query(), None);
        assert!(monitor.sample(Duration::ZERO).is_ok());
    }
    #[test]
    fn constructors_and_getters_expose_an_empty_monitor_without_allocating_history() {
        for monitor in [Monitor::default(), Monitor::new(NonZeroUsize::MIN)] {
            assert_eq!(monitor.submitted(), Totals::default());
            assert_eq!(monitor.confirmed(), Totals::default());
            assert_eq!(monitor.active_query(), None);
            assert_eq!(monitor.last_query(), None);
            assert_eq!(monitor.observed_at(), Duration::ZERO);
            assert_eq!(monitor.history().len(), 0);
            assert_eq!(monitor.history.capacity(), 0);
            assert_eq!(monitor.cohorts.capacity(), 0);
        }
    }

    #[test]
    fn submission_only_changes_totals_without_observing_time_or_allocating() {
        let mut monitor = Monitor::default();
        monitor.record_submission(Totals { events: 1, applications: 3 }).unwrap();
        monitor.record_submission(Totals { events: 63, applications: 189 }).unwrap();
        assert_eq!(monitor.submitted(), Totals { events: 64, applications: 192 });
        assert_eq!(monitor.confirmed(), Totals::default());
        assert_eq!(monitor.observed_at(), Duration::ZERO);
        assert_eq!(monitor.history.capacity(), 0);
        assert_eq!(monitor.cohorts.capacity(), 0);
    }

    #[test]
    fn query_start_and_completion_expose_the_same_prefix_and_latency() {
        let mut monitor = Monitor::default();
        let prefix = Totals { events: 5, applications: 15 };
        monitor.record_submission(prefix).unwrap();
        let query = monitor.start_query(Duration::from_millis(100)).unwrap();
        assert_eq!(query.submitted, prefix);
        assert_eq!(monitor.active_query(), Some(query));
        assert_eq!(monitor.last_query(), None);
        let observation = monitor.complete_query(query, Duration::from_millis(150)).unwrap();
        assert_eq!(observation.duration, Duration::from_millis(50));
        assert_eq!(monitor.last_query(), Some(observation));
        assert_eq!(monitor.confirmed(), prefix);
        assert_eq!(monitor.active_query(), None);
        assert_eq!(monitor.observed_at(), Duration::from_millis(150));
    }

    #[test]
    fn explicit_prefix_accepts_boundaries_and_rejects_both_sides_atomically() {
        let mut monitor = Monitor::default();
        monitor.record_submission(Totals { events: 2, applications: 6 }).unwrap();
        let query = monitor.start_query_through(Duration::ZERO, Totals::default()).unwrap();
        monitor.complete_query(query, Duration::ZERO).unwrap();
        assert_eq!(monitor.confirmed(), Totals::default());
        assert_eq!(monitor.start_query_through(Duration::ZERO, Totals { events: 3, applications: 6 }), Err(Error::InvalidPrefix));
        assert_eq!(monitor.start_query_through(Duration::ZERO, Totals { events: 2, applications: 7 }), Err(Error::InvalidPrefix));
        let query = monitor.start_query_through(Duration::ZERO, monitor.submitted()).unwrap();
        monitor.complete_query(query, Duration::ZERO).unwrap();
        assert_eq!(monitor.start_query_through(Duration::ZERO, Totals { events: 1, applications: 6 }), Err(Error::InvalidPrefix));
        assert_eq!(monitor.start_query_through(Duration::ZERO, Totals { events: 2, applications: 5 }), Err(Error::InvalidPrefix));
    }

    #[test]
    fn snapshot_reads_do_not_move_sample_baselines_or_clock() {
        let mut monitor = Monitor::default();
        monitor.sample(Duration::ZERO).unwrap();
        monitor.record_submission(Totals { events: 10, applications: 30 }).unwrap();
        let first = monitor.snapshot(Duration::from_secs(1)).unwrap();
        assert_eq!(first.recent_submitted.events_per_second, 10.0);
        assert_eq!(monitor.snapshot(Duration::from_secs(2)).unwrap().recent_submitted.events_per_second, 5.0);
        assert_eq!(monitor.observed_at(), Duration::ZERO);
        assert_eq!(monitor.history().len(), 1);
        assert_eq!(monitor.cohorts.len(), 0);
    }

    #[test]
    fn history_rollover_keeps_order_for_forward_and_reverse_iteration() {
        let mut monitor = Monitor::new(NonZeroUsize::new(2).unwrap());
        for second in 0..5 {
            monitor.sample(Duration::from_secs(second)).unwrap();
        }
        assert_eq!(monitor.history().map(|x| x.at).collect::<Vec<_>>(), [Duration::from_secs(3), Duration::from_secs(4)]);
        assert_eq!(monitor.history().rev().map(|x| x.at).collect::<Vec<_>>(), [Duration::from_secs(4), Duration::from_secs(3)]);
    }

    #[test]
    fn completion_removes_only_fully_confirmed_cohorts() {
        let mut monitor = Monitor::default();
        monitor.sample(Duration::ZERO).unwrap();
        for second in 1..=120 {
            monitor.record_submission(Totals { events: 1, applications: 3 }).unwrap();
            monitor.sample(Duration::from_secs(second)).unwrap();
        }
        let query = monitor.start_query_through(Duration::from_secs(120), Totals { events: 60, applications: 180 }).unwrap();
        monitor.complete_query(query, Duration::from_secs(120)).unwrap();
        assert_eq!(monitor.cohorts.len(), 60);
        assert_eq!(monitor.snapshot(Duration::from_secs(121)).unwrap().oldest_outstanding_age, Some(Duration::from_secs(61)));
        let query = monitor.start_query(Duration::from_secs(121)).unwrap();
        monitor.complete_query(query, Duration::from_secs(121)).unwrap();
        assert!(monitor.cohorts.is_empty());
    }
}
