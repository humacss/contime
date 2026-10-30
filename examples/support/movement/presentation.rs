//! Bounded, presentation-only movement scene shared by terminal and browser hosts.
use serde::{Deserialize, Serialize};
/// Keep the movement demonstration small enough to follow visually.
pub const MAX_CLIENTS: usize = 5;
/// Bound transport and drawing independently of the configured input rate.
pub const MAX_NETWORK_EVENTS: usize = 128;
/// Shared visible clock window, in nanoseconds: past and future of delivery.
#[allow(dead_code)] // The shared report is also included by non-movement hosts.
pub fn timeline_window(average_ms: u64, jitter_ms: u64) -> (f64, f64) {
    let delay = average_ms.saturating_add(average_ms.min(jitter_ms)) as f64 * 1_000_000.0;
    let span = 6_000_000_000.0_f64.max(delay * 7.0 / 3.0);
    (span * 4.0 / 7.0, span * 3.0 / 7.0)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    /// Segment origin. None is supported for older point-only terminal fixtures.
    #[serde(default)]
    pub previous: Option<[f64; 2]>,
    #[serde(default)]
    pub client: usize,
    #[serde(default)]
    pub time_ns: u64,
    pub position: [f64; 2],
    pub corrected: bool,
    pub opacity: f64,
    pub current: bool,
}
/// Delivered input at its historical position, not its network arrival position.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arrival {
    pub id: u64,
    pub client: usize,
    pub time_ns: u64,
    pub position: [f64; 2],
    /// Unit movement direction; [0, 0] denotes Stop.
    pub direction: [f64; 2],
    pub opacity: f64,
    /// Arrival pulse strength, independent of the historical path age.
    pub pulse: f64,
}
/// Simulated network timing, independent of ConTime's internal pending work.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkEvent {
    pub id: u64,
    pub client: usize,
    pub direction: [f64; 2],
    pub action_ns: u64,
    /// Scheduled delivery. May be overdue while an example query is pending.
    pub delivery_ns: u64,
    /// Actual handoff time; absent until take_ready removes the queued input.
    pub arrived_ns: Option<u64>,
    pub reordered: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Client {
    pub id: usize,
    /// Latest queried input direction; zero denotes Stop.
    #[serde(default)]
    pub direction: [f64; 2],
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
    #[serde(default)]
    pub arrivals: Vec<Arrival>,
    #[serde(default)]
    pub now_ns: u64,
    #[serde(default)]
    pub network: Vec<NetworkEvent>,
    pub preview_limited: bool,
}

impl Report {
    pub fn validate(&self) -> Result<(), String> {
        let finite = |p: &[f64; 2]| p.iter().all(|v| v.is_finite());
        if self.client_spacing > 1_000_000 || self.movement_speed > 1_000_000 {
            return Err("Invalid movement scale".into());
        }
        if !(1..=MAX_CLIENTS).contains(&self.clients)
            || !(1..=1024).contains(&self.workers)
            || self.positions.len() > MAX_CLIENTS
            || self.trails.len() > 4096
            || self.network.len() > MAX_NETWORK_EVENTS
            || self.network.iter().any(|e| {
                e.client == 0
                    || e.client > self.clients
                    || e.action_ns > self.now_ns
                    || e.delivery_ns < e.action_ns
                    || e.arrived_ns.is_some_and(|at| at < e.delivery_ns || at > self.now_ns)
                    || (e.reordered && e.arrived_ns.is_none())
                    || !matches!(e.direction, [0.0, 0.0] | [1.0, 0.0] | [-1.0, 0.0] | [0.0, 1.0] | [0.0, -1.0])
            })
            || self.arrivals.len() > 128
            || self.arrivals.iter().any(|a| {
                a.client == 0
                    || a.client > self.clients
                    || !finite(&a.position)
                    || !finite(&a.direction)
                    || !a.pulse.is_finite()
                    || !(0.0..=1.0).contains(&a.pulse)
                    || !a.opacity.is_finite()
                    || !(0.0..=1.0).contains(&a.opacity)
                    || !matches!(a.direction, [0.0, 0.0] | [1.0, 0.0] | [-1.0, 0.0] | [0.0, 1.0] | [0.0, -1.0])
            })
            || self.trails.iter().any(|p| p.previous.is_some_and(|v| !finite(&v) || p.client == 0 || p.client > self.clients))
            || !finite(&self.center)
            || !finite(&self.arena_half_extent)
            || self.arena_half_extent.iter().any(|extent| *extent <= 0.0)
            || self.positions.iter().any(|p| {
                p.id == 0
                    || p.id > self.clients
                    || !finite(&p.position)
                    || !matches!(p.direction, [0.0, 0.0] | [1.0, 0.0] | [-1.0, 0.0] | [0.0, 1.0] | [0.0, -1.0])
            })
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
