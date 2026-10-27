use std::{fmt, time::Duration};

/// Cumulative source event and destination-application counts, supplied by the host.
/// One event affecting three destinations counts as one event and three applications.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Totals {
    /// Source events, before destination fanout.
    pub events: u64,
    /// Event applications across destination histories.
    pub applications: u64,
}

impl Totals {
    pub(crate) fn checked_add(self, other: Self) -> Option<Self> {
        Some(Self { events: self.events.checked_add(other.events)?, applications: self.applications.checked_add(other.applications)? })
    }

    pub(crate) fn since(self, earlier: Self) -> Self {
        Self { events: self.events.saturating_sub(earlier.events), applications: self.applications.saturating_sub(earlier.applications) }
    }
}

/// A captured submission prefix. Return the entire, unchanged token to completion.
/// Tokens identify queries within one monitor; do not exchange them across monitors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Query {
    /// Monotonically increasing identity within the owning monitor.
    pub id: u64,
    /// Submission prefix captured when the query started.
    pub submitted: Totals,
    /// Caller-supplied query start time.
    pub started: Duration,
}

/// A completed query and its caller-observed latency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observation {
    /// Original query token.
    pub query: Query,
    /// Caller-supplied completion time.
    pub completed_at: Duration,
    /// Completion time minus start time.
    pub duration: Duration,
}

/// Throughput over an observation interval; zero when the interval has no duration.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Rates {
    /// Source events per second.
    pub events_per_second: f64,
    /// Destination applications per second.
    pub applications_per_second: f64,
}
impl Rates {
    pub(crate) fn between(delta: Totals, duration: Duration) -> Self {
        let seconds = duration.as_secs_f64();
        if seconds == 0.0 {
            Self::default()
        } else {
            Self { events_per_second: delta.events as f64 / seconds, applications_per_second: delta.applications as f64 / seconds }
        }
    }
}

/// A passive view of query-confirmed work; this does not measure live replay completion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snapshot {
    /// Observation time supplied by the caller.
    pub at: Duration,
    /// All recorded source submissions.
    pub submitted: Totals,
    /// Prefix confirmed by completed host queries.
    pub confirmed: Totals,
    /// Submitted minus confirmed work.
    pub outstanding: Totals,
    /// Outstanding work already present at the previous sample boundary.
    pub older_outstanding: Totals,
    /// Age measured from the oldest retained observation interval boundary.
    pub oldest_outstanding_age: Option<Duration>,
    /// Currently pending query, if any.
    pub active_query: Option<Query>,
    /// Most recent completed query.
    pub last_query: Option<Observation>,
    /// Submission throughput since the previous sample.
    pub recent_submitted: Rates,
    /// Confirmation throughput since the previous sample.
    pub recent_confirmed: Rates,
    /// Submission throughput since the first sample.
    pub average_submitted: Rates,
    /// Confirmation throughput since the first sample.
    pub average_confirmed: Rates,
}

/// Rejected monitor operations leave all state unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// A cumulative counter would overflow.
    CounterOverflow,
    /// A query is already active.
    QueryAlreadyActive,
    /// The token does not exactly match the active query.
    InvalidQuery,
    /// The requested query prefix precedes confirmation or exceeds submissions.
    InvalidPrefix,
    /// Time precedes a successful timed mutation.
    ClockReversed,
    /// All query identities have been used.
    QueryIdOverflow,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::CounterOverflow => "submission counter overflow",
            Self::QueryAlreadyActive => "a query is already active",
            Self::InvalidQuery => "query token does not match the active query",
            Self::InvalidPrefix => "query prefix lies outside confirmed and submitted bounds",
            Self::ClockReversed => "observation clock moved backwards",
            Self::QueryIdOverflow => "query identity overflow",
        })
    }
}
impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_add_sums_each_unit_and_rejects_either_overflow() {
        let totals = Totals { events: 4, applications: 9 };
        assert_eq!(totals.checked_add(Totals { events: 2, applications: 6 }), Some(Totals { events: 6, applications: 15 }));
        assert_eq!(totals.checked_add(Totals { events: u64::MAX, applications: 0 }), None);
        assert_eq!(totals.checked_add(Totals { events: 0, applications: u64::MAX }), None);
    }

    #[test]
    fn since_saturates_the_units_independently() {
        let totals = Totals { events: 4, applications: 9 };
        assert_eq!(totals.since(Totals { events: 2, applications: 6 }), Totals { events: 2, applications: 3 });
        assert_eq!(totals.since(Totals { events: 5, applications: 1 }), Totals { events: 0, applications: 8 });
        assert_eq!(totals.since(totals), Totals::default());
    }

    #[test]
    fn rates_preserve_fractional_seconds_and_handle_empty_intervals() {
        let delta = Totals { events: 3, applications: 9 };
        assert_eq!(Rates::between(delta, Duration::from_millis(500)), Rates { events_per_second: 6.0, applications_per_second: 18.0 });
        assert_eq!(Rates::between(delta, Duration::ZERO), Rates::default());
        assert_eq!(Rates::between(Totals::default(), Duration::from_secs(1)), Rates::default());
        let extreme = Rates::between(Totals { events: u64::MAX, applications: u64::MAX }, Duration::from_nanos(1));
        assert!(extreme.events_per_second.is_finite());
        assert!(extreme.applications_per_second.is_finite());
    }

    #[test]
    fn every_error_has_a_distinct_nonempty_description() {
        let errors = [
            Error::CounterOverflow,
            Error::QueryAlreadyActive,
            Error::InvalidQuery,
            Error::InvalidPrefix,
            Error::ClockReversed,
            Error::QueryIdOverflow,
        ];
        let descriptions: std::collections::HashSet<_> = errors.iter().map(ToString::to_string).collect();
        assert_eq!(descriptions.len(), errors.len());
        assert!(descriptions.iter().all(|text| !text.is_empty()));
        assert!(std::error::Error::source(&errors[0]).is_none());
    }
}
