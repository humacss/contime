use crate::report::Report;
use serde::Deserialize;

pub const MAX_LINE_BYTES: usize = 1024 * 1024;
#[derive(Deserialize)]
pub struct Frame {
    pub done: bool,
    pub movement: Report,
}
/// Decode only presentation from the existing versioned worker-report transport.
/// All generation, counters, state queries and validation remain in the workload.
pub fn decode(line: &str) -> Result<Frame, String> {
    if line.len() > MAX_LINE_BYTES {
        return Err("Movement report exceeds protocol limit".into());
    }
    let payload = line.strip_prefix("contime-workers-v2\t").ok_or("Unknown movement report protocol")?;
    let frame: Frame = serde_json::from_str(payload).map_err(|error| error.to_string())?;
    frame.movement.validate()?;
    Ok(frame)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unversioned_malformed_and_oversized_records() {
        for line in ["{}".to_owned(), "contime-workers-v2\t{}".into(), "x".repeat(MAX_LINE_BYTES + 1)] {
            assert!(decode(&line).is_err());
        }
    }
}
