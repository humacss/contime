//! Normalize workload reports for the same display in every environment.
use serde_json::Value;
#[derive(Clone, Debug, PartialEq)]
pub struct Dashboard {
    pub hash: bool,
    pub hash_summary: Option<HashSummary>,
    pub failure: Option<String>,
    pub target_rate: u64,
    pub resident_bytes: Option<u64>,
    pub reversed: Option<u64>,
    pub worker_rows: Vec<WorkerRow>,
    pub snapshot_rows: Vec<SnapshotRow>,
    pub reading_seconds: Option<f64>,
    pub produced: u64,
    pub completed: u64,
    pub outstanding: u64,
    pub queries: u64,
    pub elapsed: f64,
    pub workers: usize,
    pub snapshots: usize,
    pub hashes: Vec<(u64, String, String)>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct HashSummary {
    pub up_to_date: u64,
    pub pending: u64,
    pub failed: u64,
    pub workers: Option<Vec<WorkerHashSummary>>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct WorkerHashSummary {
    pub up_to_date: u64,
    pub pending: u64,
    pub failed: u64,
}
impl WorkerHashSummary {
    pub fn total(&self) -> Option<u64> {
        self.up_to_date.checked_add(self.pending)?.checked_add(self.failed)
    }
}
fn hash_summary(value: &Value) -> Option<HashSummary> {
    Some(HashSummary {
        up_to_date: value.get("up_to_date")?.as_u64()?,
        pending: value.get("pending")?.as_u64()?,
        failed: value.get("failed")?.as_u64()?,
        workers: match value.get("workers") {
            None | Some(Value::Null) => None,
            Some(Value::Array(values)) => Some(
                values
                    .iter()
                    .map(|item| {
                        Some(WorkerHashSummary {
                            up_to_date: item.get("up_to_date")?.as_u64()?,
                            pending: item.get("pending")?.as_u64()?,
                            failed: item.get("failed")?.as_u64()?,
                        })
                    })
                    .collect::<Option<Vec<_>>>()?,
            ),
            _ => return None,
        },
    })
}
#[derive(Clone, Debug, PartialEq)]
pub struct WorkerRow {
    pub id: u64,
    pub snapshots: u64,
    pub pending_histories: Option<u64>,
    pub inserted: Option<u64>,
    pub replayed: Option<u64>,
    pub saturated: bool,
    pub inserted_per_second: Option<f64>,
    pub replayed_per_second: Option<f64>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct SnapshotRow {
    pub id: u64,
    pub latest_count: u64,
    pub verified_count: u64,
    pub latest_value: i64,
    pub verified_value: i64,
    pub latest_hash: Option<String>,
    pub verified_hash: Option<String>,
}
fn rows(value: &Value, wire: bool) -> Vec<SnapshotRow> {
    let digest = |v: &Value| -> Option<String> {
        if let Some(s) = v.as_str() {
            return Some(s.chars().take(8).collect());
        }
        let a = v.as_array()?;
        let mut s = String::new();
        for byte in a.iter().take(4) {
            s.push_str(&format!("{:02x}", u8::try_from(byte.as_u64()?).ok()?));
        }
        Some(s)
    };
    value
        .as_array()
        .into_iter()
        .flatten()
        .take(64)
        .filter_map(|r| {
            let prefix = if wire { "produced" } else { "latest" };
            Some(SnapshotRow {
                id: r.get("id")?.as_u64()?,
                latest_count: r.get(format!("{prefix}_count").as_str())?.as_u64()?,
                verified_count: r.get("verified_count")?.as_u64()?,
                latest_value: r.get(format!("{prefix}_value").as_str())?.as_i64()?,
                verified_value: r.get("verified_value")?.as_i64()?,
                latest_hash: digest(&r[format!("{prefix}_hash").as_str()]),
                verified_hash: digest(&r["verified_hash"]),
            })
        })
        .collect()
}
impl Dashboard {
    pub fn browser(value: &Value, domain: &str) -> Result<Self, String> {
        if !["counter", "chain-hash"].contains(&domain) {
            return Err("Unknown dashboard".into());
        }
        let number = |name| value.get(name).and_then(Value::as_u64).ok_or_else(|| format!("Missing {name}"));
        let result = Self {
            hash: domain == "chain-hash",
            hash_summary: hash_summary(&value["hash_summary"]),
            failure: value["failure"].as_str().map(str::to_owned),
            resident_bytes: None,
            target_rate: value.get("target_rate").and_then(Value::as_u64).unwrap_or(0),
            reversed: value["reversed"].as_u64(),
            worker_rows: value["workers_status"]
                .as_array()
                .into_iter()
                .flatten()
                .take(1024)
                .filter_map(|w| {
                    Some(WorkerRow {
                        id: w.get("id")?.as_u64()?,
                        snapshots: w.get("snapshots")?.as_u64()?,
                        pending_histories: w["pending"].as_u64(),
                        inserted: w["total_inserted"].as_u64(),
                        replayed: w["total_replayed"].as_u64(),
                        saturated: w["saturated"].as_bool().unwrap_or(false),
                        inserted_per_second: w["avg_inserted_per_second"].as_f64(),
                        replayed_per_second: w["avg_replayed_per_second"].as_f64(),
                    })
                })
                .collect(),
            snapshot_rows: rows(&value["snapshots"], false),
            reading_seconds: None,
            produced: number("produced")?,
            completed: number("completed")?,
            outstanding: number("outstanding")?,
            queries: number("queries")?,
            elapsed: value.get("elapsed").and_then(Value::as_f64).ok_or("Missing elapsed")?,
            workers: number("workers")? as usize,
            snapshots: number("snapshot_count")? as usize,
            hashes: value
                .get("hashes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(4)
                .filter_map(|h| {
                    Some((
                        h.get("id")?.as_u64()?,
                        h.get("latest_hash")?.as_str()?.chars().take(8).collect(),
                        h.get("verified_hash")?.as_str()?.chars().take(8).collect(),
                    ))
                })
                .collect(),
        };
        result.validate()?;
        Ok(result)
    }
    pub fn wire(value: &Value) -> Result<Self, String> {
        let metrics = value.get("metrics").ok_or("Missing metrics")?;
        let number = |name| metrics.get(name).and_then(Value::as_u64).ok_or_else(|| format!("Missing {name}"));
        let elapsed = metrics.get("elapsed").ok_or("Missing elapsed")?;
        let seconds = elapsed.get("secs").and_then(Value::as_u64).ok_or("Missing seconds")? as f64
            + elapsed.get("nanos").and_then(Value::as_u64).ok_or("Missing nanos")? as f64 / 1e9;
        let result = Self {
            hash: metrics.get("reversed").is_some_and(Value::is_number),
            hash_summary: hash_summary(&value["hash_summary"]),
            failure: value["failure"].as_str().map(str::to_owned),
            resident_bytes: None,
            target_rate: metrics.get("target_rate").and_then(Value::as_u64).unwrap_or(0),
            reversed: metrics["reversed"].as_u64(),
            worker_rows: value["reading"]["workers"]
                .as_array()
                .into_iter()
                .flatten()
                .take(1024)
                .filter_map(|w| {
                    let counters = w["counters"].as_array();
                    Some(WorkerRow {
                        id: w.get("id")?.as_u64()?,
                        snapshots: w.get("snapshots")?.as_u64()?,
                        pending_histories: w["pending"].as_u64(),
                        inserted: counters.and_then(|c| c.first()).and_then(Value::as_u64),
                        replayed: counters.and_then(|c| c.get(1)).and_then(Value::as_u64),
                        saturated: counters.and_then(|c| c.get(2)).and_then(Value::as_bool).unwrap_or(false),
                        inserted_per_second: None,
                        replayed_per_second: None,
                    })
                })
                .collect(),
            snapshot_rows: rows(&value["snapshots"], true),
            reading_seconds: value["reading"]["completed"]["secs"]
                .as_u64()
                .map(|sec| sec as f64 + value["reading"]["completed"]["nanos"].as_u64().unwrap_or(0) as f64 / 1e9),
            produced: number("produced")?,
            completed: number("completed")?,
            outstanding: number("outstanding")?,
            queries: number("queries")?,
            elapsed: seconds,
            workers: value.get("reading").and_then(|r| r.get("workers")).and_then(Value::as_array).map_or(0, Vec::len),
            snapshots: value.get("snapshot_count").and_then(Value::as_u64).ok_or("Missing snapshot count")? as usize,
            hashes: value
                .get("snapshots")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(4)
                .filter_map(|h| {
                    let hash = |name| {
                        h.get(name)?
                            .as_array()
                            .map(|bytes| bytes.iter().take(4).filter_map(Value::as_u64).map(|b| format!("{b:02x}")).collect::<String>())
                    };
                    Some((h.get("id")?.as_u64()?, hash("produced_hash")?, hash("verified_hash")?))
                })
                .collect(),
        };
        result.validate()?;
        Ok(result)
    }
    pub fn calculate_rates(&mut self, baseline: &Dashboard) {
        let Some(dt) = self.reading_seconds.zip(baseline.reading_seconds).map(|(now, start)| now - start).filter(|dt| *dt > 0.0) else {
            return;
        };
        for row in &mut self.worker_rows {
            row.inserted_per_second = None;
            row.replayed_per_second = None;
            if let Some(old) = baseline.worker_rows.iter().find(|w| w.id == row.id && !w.saturated && !row.saturated) {
                row.inserted_per_second = row.inserted.zip(old.inserted).and_then(|(a, b)| a.checked_sub(b)).map(|n| n as f64 / dt);
                row.replayed_per_second = row.replayed.zip(old.replayed).and_then(|(a, b)| a.checked_sub(b)).map(|n| n as f64 / dt);
            }
        }
    }
    fn validate(&self) -> Result<(), String> {
        if self
            .hash_summary
            .as_ref()
            .is_some_and(|s| s.up_to_date.checked_add(s.pending).and_then(|n| n.checked_add(s.failed)) != Some(self.snapshots as u64))
        {
            return Err("Invalid hash summary".into());
        }
        if let Some(summary) = &self.hash_summary
            && let Some(workers) = &summary.workers
        {
            let sums = workers.iter().try_fold((0_u64, 0_u64, 0_u64), |(up, pending, failed), w| {
                Some((up.checked_add(w.up_to_date)?, pending.checked_add(w.pending)?, failed.checked_add(w.failed)?))
            });
            if workers.len() != self.workers || sums != Some((summary.up_to_date, summary.pending, summary.failed)) {
                return Err("Invalid per-worker hash summary".into());
            }
        }
        if !self.elapsed.is_finite()
            || self.elapsed < 0.0
            || self.completed > self.produced
            || self.produced - self.completed != self.outstanding
        {
            return Err("Invalid dashboard metrics".into());
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hash_summary_uses_all_histories_and_keeps_failure_message() {
        let report = serde_json::json!({"produced": 5, "completed": 4, "outstanding": 1, "queries": 2, "elapsed": 1.0, "workers": 1, "snapshot_count": 100, "hash_summary": {"up_to_date": 78, "pending": 20, "failed": 2}, "failure": "Snapshot 80 differs"});
        let dashboard = Dashboard::browser(&report, "chain-hash").unwrap();
        assert_eq!(dashboard.hash_summary, Some(HashSummary { up_to_date: 78, pending: 20, failed: 2, workers: None }));
        assert_eq!(dashboard.failure.as_deref(), Some("Snapshot 80 differs"));
        let mut invalid = report;
        invalid["hash_summary"]["failed"] = 3.into();
        assert!(Dashboard::browser(&invalid, "chain-hash").is_err());
    }
    #[test]
    fn parses_and_validates_per_worker_verification() {
        let mut report = serde_json::json!({"produced": 3, "completed": 3, "outstanding": 0, "queries": 1, "elapsed": 2.0, "workers": 2, "snapshot_count": 5, "hash_summary": {"up_to_date": 3, "pending": 1, "failed": 1, "workers": [{"up_to_date": 1, "pending": 1, "failed": 0}, {"up_to_date": 2, "pending": 0, "failed": 1}]}});
        let summary = Dashboard::browser(&report, "chain-hash").unwrap().hash_summary.unwrap();
        assert_eq!(summary.workers.as_ref().unwrap()[0].up_to_date, 1);
        report["hash_summary"]["workers"][0]["up_to_date"] = 3.into();
        assert!(Dashboard::browser(&report, "chain-hash").is_err());
    }
    #[test]
    fn browser_preserves_worker_activity_and_counter_snapshot_preview() {
        let value = serde_json::json!({"produced": 10, "completed": 8, "outstanding": 2, "queries": 1, "elapsed": 2.0, "workers": 1, "snapshot_count": 1, "target_rate": 1000,
            "workers_status": [{"id": 0, "snapshots": 1, "pending": 2, "total_inserted": 12, "total_replayed": 3, "avg_inserted_per_second": 5.0, "avg_replayed_per_second": 1.5}],
            "snapshots": [{"id": 1, "latest_count": 8, "verified_count": 5, "latest_value": 8, "verified_value": 5}]});
        let dashboard = Dashboard::browser(&value, "counter").unwrap();
        assert_eq!(dashboard.target_rate, 1000);
        assert_eq!(dashboard.worker_rows[0].inserted, Some(12));
        assert_eq!(dashboard.worker_rows[0].pending_histories, Some(2));
        assert_eq!(dashboard.worker_rows[0].replayed, Some(3));
        assert_eq!(dashboard.snapshot_rows[0].latest_value, 8);
        assert_eq!(dashboard.snapshot_rows[0].verified_value, 5);
    }
    #[test]
    fn wire_preserves_worker_counters_and_snapshot_values() {
        let value = serde_json::json!({"done": false, "metrics": {"target_rate": 1000, "produced": 10, "completed": 8, "outstanding": 2, "queries": 1, "elapsed": {"secs": 2, "nanos": 0}, "reversed": null}, "reading": {"completed": {"secs": 2, "nanos": 0}, "workers": [{"id": 0, "snapshots": 1, "pending": 2, "counters": [12, 3, false]}]}, "snapshot_count": 1, "snapshots": [{"id": 1, "produced_count": 8, "verified_count": 5, "produced_value": 8, "verified_value": 5, "produced_hash": null, "verified_hash": null}]});
        let mut report = Dashboard::wire(&value).unwrap();
        assert_eq!(report.worker_rows[0].inserted, Some(12));
        assert_eq!(report.worker_rows[0].pending_histories, Some(2));
        assert_eq!(report.snapshot_rows[0].latest_value, 8);
        assert_eq!(report.snapshot_rows[0].verified_value, 5);
        let mut baseline = report.clone();
        baseline.reading_seconds = Some(1.0);
        baseline.worker_rows[0].inserted = Some(2);
        baseline.worker_rows[0].replayed = Some(1);
        report.calculate_rates(&baseline);
        assert_eq!(report.worker_rows[0].inserted_per_second, Some(10.0));
        assert_eq!(report.worker_rows[0].replayed_per_second, Some(2.0));
        report.worker_rows[0].saturated = true;
        report.calculate_rates(&baseline);
        assert_eq!(report.worker_rows[0].inserted_per_second, None);
    }
    #[test]
    fn both_hosts_present_the_same_counters_and_reject_inconsistent_counts() {
        let browser = serde_json::json!({"produced": 10, "completed": 8, "outstanding": 2, "queries": 1, "elapsed": 1.5, "workers": 1, "snapshot_count": 4});
        let wire = serde_json::json!({"metrics": {"produced": 10, "completed": 8, "outstanding": 2, "queries": 1, "elapsed": {"secs": 1, "nanos": 500000000}, "reversed": null}, "reading": {"workers": [{}]}, "snapshot_count": 4});
        assert_eq!(Dashboard::browser(&browser, "counter").unwrap(), Dashboard::wire(&wire).unwrap());
        let hash = serde_json::json!({"produced": 1, "completed": 1, "outstanding": 0, "queries": 1, "elapsed": 1.0, "workers": 1, "snapshot_count": 1, "hashes": [{"id": 1, "latest_hash": "0123456789abcdef", "verified_hash": "0123456799abcdef"}]});
        let row = Dashboard::browser(&hash, "chain-hash").unwrap().hashes;
        assert_eq!(row, vec![(1, "01234567".into(), "01234567".into())]);
        let mut invalid = browser;
        invalid["outstanding"] = 3.into();
        assert!(Dashboard::browser(&invalid, "counter").is_err());
    }
}
