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
}
impl Metrics {
    pub fn text(&self) -> String {
        let number = crate::support::number;
        let rate = crate::support::decimal(self.completed as f64 / self.elapsed.as_secs_f64().max(f64::EPSILON), 0);
        let mut text = format!("Target {:>15} events/s | Average {:>15} completed events/s\nOutstanding {:>10} unconfirmed events\nProduced {:>15} | Confirmed {:>15}",
            number(self.target_rate), rate, number(self.outstanding), number(self.produced), number(self.completed));
        if let Some(reversed) = self.reversed {
            text.push_str(&format!("\nReversed arrivals {:>12} | Hash queries {:>10}", number(reversed), number(self.queries)));
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
