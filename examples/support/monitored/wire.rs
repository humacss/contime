//! Bounded example-only protocol: WebAssembly computes, the launcher presents.
pub use crate::support::monitored_metrics::SnapshotPair;
use crate::{
    definition::{Example, Time},
    support::monitored_metrics::Metrics,
};
use contime_monitor::{WorkerCounters, WorkerReading, WorkerStatus};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};
const MAGIC: &str = "contime-workers-v2\t";
pub const MAX_LINE_BYTES: usize = 1024 * 1024;
pub const MAX_SNAPSHOT_PREVIEW: usize = 64;
pub struct Sample {
    pub metrics: Metrics,
    pub reading: Option<Arc<WorkerReading<Time>>>,
    pub done: bool,
    pub snapshot_count: usize,
    pub snapshots: Vec<SnapshotPair>,
    pub movement: Option<crate::support::movement_report::Report>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    metrics: Metrics,
    reading: Option<Reading>,
    done: bool,
    snapshot_count: usize,
    snapshots: Vec<SnapshotPair>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    movement: Option<crate::support::movement_report::Report>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reading {
    started: Duration,
    completed: Duration,
    workers: Vec<Worker>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Worker {
    id: usize,
    snapshots: usize,
    metadata: usize,
    pending: usize,
    due: usize,
    target: u64,
    horizon: u64,
    pruning: usize,
    round: Option<u64>,
    // All counts remain integers; unknown replay and saturation remain explicit.
    counters: Option<(u64, Option<u64>, bool)>,
}
impl Sample {
    pub fn new<M: Example>(
        metrics: &Metrics,
        reading: Option<&Arc<WorkerReading<Time>>>,
        expected: &[M::State],
        verified: &[M::State],
        done: bool,
    ) -> Self {
        Self {
            movement: None,
            metrics: metrics.clone(),
            reading: reading.cloned(),
            done,
            snapshot_count: expected.len(),
            snapshots: expected
                .iter()
                .zip(verified)
                .take(MAX_SNAPSHOT_PREVIEW)
                .enumerate()
                .map(|(index, (produced, verified))| SnapshotPair::new::<M>(index, produced, verified))
                .collect(),
        }
    }

    pub fn encode(&self) -> String {
        let reading = self.reading.as_ref().map(|r| Reading {
            started: r.started_at,
            completed: r.completed_at,
            workers: r
                .workers
                .iter()
                .map(|w| Worker {
                    id: w.worker_id,
                    snapshots: w.history_stores,
                    metadata: w.metadata_only,
                    pending: w.pending_histories,
                    due: w.due_histories,
                    target: w.processing_target.0,
                    horizon: w.admission_horizon.0,
                    pruning: w.pending_prune_passes,
                    round: w.awaiting_resolution,
                    counters: w.counters.map(|c| (c.inserted_applications, c.replay_applications, c.saturated)),
                })
                .collect(),
        });
        format!(
            "{MAGIC}{}",
            serde_json::to_string(&Record {
                metrics: self.metrics.clone(),
                reading,
                done: self.done,
                snapshot_count: self.snapshot_count,
                snapshots: self.snapshots.clone(),
                movement: self.movement.clone()
            })
            .expect("integer metrics serialize")
        )
    }
    pub fn decode(line: &str) -> Result<Self, String> {
        if line.len() > MAX_LINE_BYTES {
            return Err("WASI worker record exceeds its size limit".into());
        }
        let record: Record = serde_json::from_str(line.strip_prefix(MAGIC).ok_or("Invalid WASI worker protocol/version")?)
            .map_err(|e| format!("Invalid WASI worker record: {e}"))?;
        if record.snapshot_count > 1_000_000
            || record.snapshots.len() > MAX_SNAPSHOT_PREVIEW
            || record.snapshots.len() > record.snapshot_count
            || record.snapshots.iter().enumerate().any(|(i, s)| s.id != i + 1 || s.verified_count > s.produced_count)
            || (record.done
                && record.snapshots.iter().any(|s| {
                    s.produced_count != s.verified_count || s.produced_value != s.verified_value || s.produced_hash != s.verified_hash
                }))
        {
            return Err("Invalid WASI snapshot preview".into());
        }
        if let Some(movement) = &record.movement {
            movement.validate()?;
            if movement.clients != record.snapshot_count
                || movement.submitted != record.metrics.produced
                || (record.done && movement.in_flight != 0)
            {
                return Err("Inconsistent movement report".into());
            }
        }
        let m = &record.metrics;
        if m.completed > m.produced || m.outstanding != m.produced - m.completed || (record.done && m.outstanding != 0) {
            return Err("Inconsistent WASI source-event metrics".into());
        }
        let reading = record
            .reading
            .map(|r| {
                if r.completed < r.started
                    || r.completed > m.elapsed
                    || r.workers.is_empty()
                    || r.workers.len() > 1024
                    || r.workers.iter().enumerate().any(|(i, w)| w.id != i || w.horizon > w.target)
                {
                    return Err("Invalid WASI worker reading".to_string());
                }
                Ok(Arc::new(WorkerReading {
                    started_at: r.started,
                    completed_at: r.completed,
                    workers: r
                        .workers
                        .into_iter()
                        .map(|w| WorkerStatus {
                            worker_id: w.id,
                            history_stores: w.snapshots,
                            metadata_only: w.metadata,
                            pending_histories: w.pending,
                            due_histories: w.due,
                            processing_target: Time(w.target),
                            admission_horizon: Time(w.horizon),
                            pending_prune_passes: w.pruning,
                            awaiting_resolution: w.round,
                            counters: w.counters.map(|(inserted_applications, replay_applications, saturated)| WorkerCounters {
                                inserted_applications,
                                replay_applications,
                                saturated,
                            }),
                        })
                        .collect(),
                }))
            })
            .transpose()?;
        Ok(Self {
            movement: record.movement,
            metrics: record.metrics,
            reading,
            done: record.done,
            snapshot_count: record.snapshot_count,
            snapshots: record.snapshots,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn worker_transport_preserves_large_counts_unknown_replay_and_observation_clock() {
        let sample = Sample {
            movement: None,
            snapshot_count: 0,
            snapshots: Vec::new(),
            done: false,
            metrics: Metrics {
                target_rate: 100,
                produced: 9,
                completed: 7,
                outstanding: 2,
                elapsed: Duration::from_secs(4),
                reversed: None,
                queries: 0,
            },
            reading: Some(Arc::new(WorkerReading {
                started_at: Duration::from_secs(2),
                completed_at: Duration::from_secs(3),
                workers: vec![WorkerStatus {
                    worker_id: 0,
                    history_stores: 17,
                    metadata_only: 1,
                    pending_histories: 2,
                    due_histories: 2,
                    processing_target: Time(99),
                    admission_horizon: Time(50),
                    pending_prune_passes: 1,
                    awaiting_resolution: None,
                    counters: Some(WorkerCounters { inserted_applications: u64::MAX, replay_applications: None, saturated: true }),
                }],
            })),
        };
        let decoded = Sample::decode(&sample.encode()).unwrap();
        assert_eq!(decoded.metrics.produced, 9);
        let reading = decoded.reading.unwrap();
        assert_eq!(reading.completed_at, Duration::from_secs(3));
        assert_eq!(reading.workers[0].counters.unwrap().inserted_applications, u64::MAX);
        assert_eq!(reading.workers[0].counters.unwrap().replay_applications, None);
        assert!(reading.workers[0].counters.unwrap().saturated);
        assert!(Sample::decode(&sample.encode().replace("\"done\":false", "\"done\":true")).is_err());
        assert!(Sample::decode("garbage").is_err());
        assert!(Sample::decode(&"x".repeat(MAX_LINE_BYTES + 1)).is_err());
        assert!(Sample::decode(&sample.encode().replace("\"id\":0", "\"id\":1")).is_err());
    }
}

#[cfg(test)]
mod preview_tests {
    use super::*;
    #[test]
    fn hash_previews_are_bounded_and_final_records_require_verified_full_state() {
        let pair = SnapshotPair {
            id: 1,
            produced_count: 2,
            verified_count: 2,
            produced_value: 3,
            verified_value: 3,
            produced_hash: Some([7; 32]),
            verified_hash: Some([7; 32]),
        };
        let mut sample = Sample {
            movement: None,
            metrics: Metrics::default(),
            reading: None,
            done: true,
            snapshot_count: 1_000_000,
            snapshots: vec![pair.clone()],
        };
        let decoded = Sample::decode(&sample.encode()).unwrap();
        assert_eq!(decoded.snapshot_count, 1_000_000);
        assert_eq!(decoded.snapshots.as_slice(), std::slice::from_ref(&pair));
        sample.snapshots[0].verified_hash.as_mut().unwrap()[31] ^= 1;
        assert!(Sample::decode(&sample.encode()).is_err());
        sample.done = false;
        assert!(Sample::decode(&sample.encode()).is_ok());
        sample.snapshots = (0..=MAX_SNAPSHOT_PREVIEW).map(|i| SnapshotPair { id: i + 1, ..pair.clone() }).collect();
        assert!(Sample::decode(&sample.encode()).is_err());
    }
}
