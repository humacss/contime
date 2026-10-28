//! Counter deltas use elapsed observation time, never application timestamps.
use crate::contime_monitor::WorkerReading;

/// Counts and observed rates for one worker. Missing data is never synthetic zero.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkerActivity {
    /// Stable worker ID from its status report.
    pub worker_id: usize,
    /// Cumulative canonical event-to-history insertions.
    pub inserted_applications: Option<u64>,
    /// Cumulative raw live replay traversals, including repeated replay.
    pub replay_applications: Option<u64>,
    /// Observed insertion rate, unavailable before a valid baseline or after reset.
    pub inserted_per_second: Option<f64>,
    /// Observed live replay rate, unavailable for uncounted stores.
    pub replay_per_second: Option<f64>,
    /// A worker counter saturated; totals are lower bounds and rates unavailable.
    pub saturated: bool,
}
/// Application totals and rates derived from complete immutable worker readings.
///
/// One source event targeting three histories counts as three applications.
/// Rates are observations over host completion-clock intervals, with collection
/// delay as sampling uncertainty. They do not measure unique source events or
/// outstanding work. No engine queries, clocks, or shared counters are used.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ApplicationActivity {
    /// Sum across all workers, unavailable if any worker lacks instrumentation.
    pub inserted_applications: Option<u128>,
    /// Sum across all workers, unavailable if any replay count is unknown.
    pub replay_applications: Option<u128>,
    /// Aggregate observed rate; requires the same worker set and valid baselines.
    pub inserted_per_second: Option<f64>,
    /// Aggregate observed live replay rate.
    pub replay_per_second: Option<f64>,
    /// At least one cumulative counter is a saturated lower bound.
    pub saturated: bool,
    /// Worker observations in the current reading's order.
    pub workers: Vec<WorkerActivity>,
}
impl ApplicationActivity {
    /// Derives counts and rates from sequential complete readings.
    ///
    /// Readings must belong to the same runtime instance; start a new baseline
    /// when replacing the source. Collector readings are sorted by worker ID. Membership/order changes
    /// invalidate aggregate rates; unmatched rows have no baseline. Equal or
    /// backwards clocks and decreased counters produce unavailable rates.
    /// An empty worker set has no aggregate counters. Nothing is mutated.
    pub fn between<T>(previous: Option<&WorkerReading<T>>, current: &WorkerReading<T>) -> Self {
        let seconds =
            previous.and_then(|old| current.completed_at.checked_sub(old.completed_at)).filter(|d| !d.is_zero()).map(|d| d.as_secs_f64());
        let workers: Vec<_> = current
            .workers
            .iter()
            .enumerate()
            .map(|(index, status)| {
                let mut activity = WorkerActivity { worker_id: status.worker_id, ..WorkerActivity::default() };
                let Some(counts) = status.counters else { return activity };
                activity.inserted_applications = Some(counts.inserted_applications);
                activity.replay_applications = counts.replay_applications;
                activity.saturated = counts.saturated;
                let old = previous
                    .and_then(|old| old.workers.get(index))
                    .filter(|old| old.worker_id == status.worker_id)
                    .and_then(|old| old.counters);
                if let (Some(old), Some(seconds)) = (old, seconds) {
                    if !counts.saturated
                        && !old.saturated
                        && counts.inserted_applications >= old.inserted_applications
                        && counts.replay_applications.zip(old.replay_applications).is_none_or(|(new, old)| new >= old)
                    {
                        activity.inserted_per_second =
                            counts.inserted_applications.checked_sub(old.inserted_applications).map(|n| n as f64 / seconds);
                        activity.replay_per_second = counts
                            .replay_applications
                            .zip(old.replay_applications)
                            .and_then(|(now, old)| now.checked_sub(old))
                            .map(|n| n as f64 / seconds);
                    }
                }
                activity
            })
            .collect();
        if workers.is_empty() {
            return Self::default();
        }
        let comparable = previous.is_some_and(|old| {
            old.workers.len() == workers.len() && old.workers.iter().zip(&workers).all(|(old, now)| old.worker_id == now.worker_id)
        });
        Self {
            inserted_applications: workers.iter().try_fold(0u128, |sum, w| Some(sum + u128::from(w.inserted_applications?))),
            replay_applications: workers.iter().try_fold(0u128, |sum, w| Some(sum + u128::from(w.replay_applications?))),
            inserted_per_second: if comparable { workers.iter().try_fold(0.0, |sum, w| Some(sum + w.inserted_per_second?)) } else { None },
            replay_per_second: if comparable { workers.iter().try_fold(0.0, |sum, w| Some(sum + w.replay_per_second?)) } else { None },
            saturated: workers.iter().any(|w| w.saturated),
            workers,
        }
    }
}
