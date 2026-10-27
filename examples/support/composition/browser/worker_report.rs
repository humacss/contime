//! Presentation-neutral worker totals shared by every browser case.
use crate::definition::Time;
use contime_monitor::{ApplicationActivity, WorkerReading};
pub fn workers(baseline: Option<&WorkerReading<Time>>, latest: Option<&WorkerReading<Time>>) -> Vec<serde_json::Value> {
    let average = latest.map(|r| ApplicationActivity::between(baseline, r));
    let workers = latest
        .as_ref()
        .map(|r| {
            r.workers
                .iter()
                .enumerate()
                .map(|(index, w)| {
                    let activity = &average.as_ref().unwrap().workers[index];
                    serde_json::json!({ "id": w.worker_id, "snapshots": w.history_stores,
                "pending": w.pending_histories, "due": w.due_histories, "target": w.processing_target.0, "horizon": w.admission_horizon.0,
                "total_inserted": activity.inserted_applications, "total_replayed": activity.replay_applications,
                "avg_inserted_per_second": activity.inserted_per_second, "avg_replayed_per_second": activity.replay_per_second })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    workers
}
