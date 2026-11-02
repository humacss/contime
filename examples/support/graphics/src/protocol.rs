use crate::report::Report;
use serde::Deserialize;

pub const MAX_LINE_BYTES: usize = 1024 * 1024;
#[derive(Deserialize)]
pub struct Frame {
    pub done: bool,
    pub movement: Option<Report>,
    pub uptime: Option<crate::uptime_report::Report>,
    pub accounts: Option<crate::accounts_report::Report>,
    pub robotics: Option<crate::robotics_report::Report>,
}
/// Decode only presentation from the existing versioned worker-report transport.
/// All generation, counters, state queries and validation remain in the workload.
pub fn decode(line: &str) -> Result<Frame, String> {
    if line.len() > MAX_LINE_BYTES {
        return Err("Presentation report exceeds protocol limit".into());
    }
    let payload = line.strip_prefix("contime-workers-v2\t").ok_or("Unknown presentation report protocol")?;
    let frame: Frame = serde_json::from_str(payload).map_err(|error| error.to_string())?;
    match (&frame.movement, &frame.uptime, &frame.accounts, &frame.robotics) {
        (Some(report), None, None, None) => report.validate()?,
        (None, Some(report), None, None) => report.validate()?,
        (None, None, Some(report), None) => report.validate()?,
        (None, None, None, Some(report)) => report.validate()?,
        _ => return Err("Expected exactly one visualization report".into()),
    }
    Ok(frame)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_legacy_movement_and_exclusive_uptime_but_rejects_ambiguous_or_invalid_frames() {
        let movement = crate::animation::tests::scene(0.0);
        let uptime = crate::uptime_report::Report::default();
        let decode_json = |value| decode(&format!("contime-workers-v2\t{value}"));
        assert!(decode_json(serde_json::json!({"done": false, "movement": movement})).unwrap().movement.is_some());
        assert!(decode_json(serde_json::json!({"done": true, "uptime": uptime})).unwrap().uptime.is_some());
        assert!(decode_json(serde_json::json!({"done": true, "movement": movement, "uptime": uptime})).is_err());
        assert!(decode_json(serde_json::json!({"done": true, "movement": null, "uptime": null})).is_err());
        let robotics = crate::robotics_report::Report::default();
        assert!(decode_json(serde_json::json!({"done":true,"robotics":robotics})).unwrap().robotics.is_some());
        assert!(decode_json(serde_json::json!({"done":false,"robotics":robotics,"uptime":uptime})).is_err());
        let accounts = crate::accounts_report::Report::default();
        assert!(decode_json(serde_json::json!({"done":true,"accounts":accounts})).unwrap().accounts.is_some());
        assert!(decode_json(serde_json::json!({"done":true,"accounts":accounts,"uptime":uptime})).is_err());
        let invalid = crate::uptime_report::Report { window_start_ms: 1, ..uptime };
        assert!(decode_json(serde_json::json!({"done": true, "uptime": invalid})).is_err());
    }
    #[test]
    fn rejects_unversioned_malformed_and_oversized_records() {
        for line in ["{}".to_owned(), "contime-workers-v2\t{}".into(), "x".repeat(MAX_LINE_BYTES + 1)] {
            assert!(decode(&line).is_err());
        }
    }
}
