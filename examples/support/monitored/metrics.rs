//! Presentation-neutral source-event metrics from the shared monitored loop.
use std::time::Duration;
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metrics {
    pub target_rate: u64,
    pub produced: u64,
    pub completed: u64,
    pub outstanding: u64,
    pub elapsed: Duration,
    pub reversed: Option<u64>,
    pub queries: u64,
    #[serde(default)]
    pub verified_once: bool,
}
impl Metrics {
    pub fn text(&self) -> String {
        let number = crate::support::number;
        let rate = crate::support::decimal(self.completed as f64 / self.elapsed.as_secs_f64().max(f64::EPSILON), 0);
        let mut text = format!(
            "Target {:>15} events/s | Average {:>15} completed events/s\nOutstanding {:>10} unconfirmed events\nProduced {:>15} | Confirmed {:>15}",
            number(self.target_rate),
            rate,
            number(self.outstanding),
            number(self.produced),
            number(self.completed)
        );
        if let Some(reversed) = self.reversed {
            text.push_str(&format!("\nReordered transitions {:>12} | Hash queries {:>10}", number(reversed), number(self.queries)));
        }
        text
    }
}

/// Presentation-neutral comparison of one generated and last verified snapshot.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotPair {
    pub id: usize,
    pub produced_count: u64,
    pub verified_count: u64,
    pub produced_value: i64,
    pub verified_value: i64,
    pub produced_hash: Option<[u8; 32]>,
    pub verified_hash: Option<[u8; 32]>,
}
impl SnapshotPair {
    pub fn new<M: crate::definition::Example>(index: usize, produced: &M::State, verified: &M::State) -> Self {
        Self {
            id: index + 1,
            produced_count: M::count(produced),
            verified_count: M::count(verified),
            produced_value: M::value(produced),
            verified_value: M::value(verified),
            produced_hash: M::hash(produced),
            verified_hash: M::hash(verified),
        }
    }
}

/// Current status of the full set of independently validated hash histories.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HashSummary {
    pub up_to_date: u64,
    pub pending: u64,
    pub failed: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workers: Option<Vec<WorkerHashSummary>>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerHashSummary {
    pub up_to_date: u64,
    pub pending: u64,
    pub failed: u64,
}
impl HashSummary {
    pub fn compare<M: crate::definition::Example>(expected: &[M::State], verified: &[M::State], failure: bool) -> Self {
        Self::compare_workers::<M>(expected, verified, failure, true, None)
    }

    pub fn compare_workers<M: crate::definition::Example>(
        expected: &[M::State],
        verified: &[M::State],
        failure: bool,
        verified_once: bool,
        worker_count: Option<usize>,
    ) -> Self {
        let mut summary = Self::default();
        let mut workers = worker_count.filter(|&count| (1..=1024).contains(&count)).map(|count| vec![WorkerHashSummary::default(); count]);
        for (index, (expected, verified)) in expected.iter().zip(verified).enumerate() {
            let same = verified_once
                && M::count(expected) == M::count(verified)
                && M::value(expected) == M::value(verified)
                && M::hash(expected) == M::hash(verified);
            let pending = !verified_once || (!failure && M::count(verified) < M::count(expected));
            let worker = workers.as_mut().map(|workers| {
                // The example's Runtime uses the default Placement, and Manual has one worker.
                let id = index as u128 + 1;
                #[cfg(feature = "runtime")]
                let owner = contime::Placement::default().worker_index(id, workers.len());
                #[cfg(not(feature = "runtime"))]
                let owner = (id % workers.len() as u128) as usize;
                &mut workers[owner]
            });
            if same {
                summary.up_to_date += 1;
                if let Some(worker) = worker {
                    worker.up_to_date += 1;
                }
            } else if pending {
                summary.pending += 1;
                if let Some(worker) = worker {
                    worker.pending += 1;
                }
            } else {
                summary.failed += 1;
                if let Some(worker) = worker {
                    worker.failed += 1;
                }
            }
        }
        summary.workers = workers;
        summary
    }
}
