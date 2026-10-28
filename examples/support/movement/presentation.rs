//! Bounded, presentation-only movement scene shared by terminal and browser hosts.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub position: [f64; 2],
    pub corrected: bool,
    pub opacity: f64,
    pub current: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Client {
    pub id: usize,
    pub position: [f64; 2],
    pub corrected: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub clients: usize,
    pub workers: usize,
    pub client_spacing: u64,
    pub movement_speed: u64,
    pub average_latency_ms: u64,
    pub jitter_ms: u64,
    pub submitted: u64,
    pub in_flight: usize,
    pub late: u64,
    pub corrections: u64,
    pub updates: u64,
    pub keyboard_inputs: u64,
    pub center: [f64; 2],
    pub arena_half_extent: [f64; 2],
    pub positions: Vec<Client>,
    pub trails: Vec<Point>,
    pub preview_limited: bool,
}

impl Report {
    pub fn validate(&self) -> Result<(), String> {
        let finite = |p: &[f64; 2]| p.iter().all(|v| v.is_finite());
        if self.client_spacing > 1_000_000 || self.movement_speed > 1_000_000 {
            return Err("Invalid movement scale".into());
        }
        if !(1..=10000).contains(&self.clients)
            || !(1..=1024).contains(&self.workers)
            || self.positions.len() > 2048
            || self.trails.len() > 4096
            || !finite(&self.center)
            || !finite(&self.arena_half_extent)
            || self.arena_half_extent.iter().any(|extent| *extent <= 0.0)
            || self.positions.iter().any(|p| p.id == 0 || p.id > self.clients || !finite(&p.position))
            || self.trails.iter().any(|p| !finite(&p.position) || !p.opacity.is_finite() || !(0.0..=1.0).contains(&p.opacity))
        {
            return Err("Invalid or oversized movement presentation".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        if self.positions.iter().any(|p| !seen.insert(p.id)) {
            return Err("Duplicate movement presentation client".into());
        }
        Ok(())
    }
}
