use crate::report::Report;
use serde::Deserialize;

pub const MAX_LINE_BYTES: usize = 1024 * 1024;
#[derive(Deserialize)]
pub struct Frame {
    pub done: bool,
    pub movement: Option<Report>,
    #[serde(skip)]
    pub dashboard: Option<crate::dashboard::Dashboard>,
}
/// Decode only presentation from the existing versioned worker-report transport.
/// All generation, counters, state queries and validation remain in the workload.
pub fn decode(line: &str) -> Result<Frame, String> {
    if line.len() > MAX_LINE_BYTES {
        return Err("Movement report exceeds protocol limit".into());
    }
    let payload = line.strip_prefix("contime-workers-v2\t").ok_or("Unknown movement report protocol")?;
    let value: serde_json::Value = serde_json::from_str(payload).map_err(|error| error.to_string())?;
    let mut frame: Frame = serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
    if let Some(movement) = &frame.movement {
        movement.validate()?;
    } else {
        frame.dashboard = Some(crate::dashboard::Dashboard::wire(&value)?);
    }
    Ok(frame)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_non_movement_worker_reports() {
        let payload = serde_json::json!({"done": false, "metrics": {"produced": 5, "completed": 4, "outstanding": 1, "queries": 2, "elapsed": {"secs": 1, "nanos": 0}, "reversed": 2}, "reading": null, "snapshot_count": 3, "snapshots": []});
        let frame = decode(&format!("contime-workers-v2\t{payload}")).unwrap();
        let dashboard = frame.dashboard.unwrap();
        assert!(dashboard.hash);
        assert_eq!((dashboard.produced, dashboard.completed, dashboard.outstanding), (5, 4, 1));
    }
    #[test]
    fn rejects_unversioned_malformed_and_oversized_records() {
        for line in ["{}".to_owned(), "contime-workers-v2\t{}".into(), "x".repeat(MAX_LINE_BYTES + 1)] {
            assert!(decode(&line).is_err());
        }
    }
}
