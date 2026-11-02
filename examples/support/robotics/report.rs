//! Bounded, data-only graphics contract; no execution or application dependencies.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Pose {
    pub x: f64,
    pub y: f64,
    pub heading: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub time_ns: u64,
    pub pose: Pose,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Estimate {
    pub pose: Pose,
    pub proposal: String,
    pub route: Vec<Pose>,
    pub observations: u64,
    pub trail: Vec<Sample>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Observation {
    pub id: u64,
    pub capture_ms: u64,
    pub due_ms: u64,
    pub arrived_ms: Option<u64>,
    pub pose: Pose,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Report {
    pub clock_ms: u64,
    pub query_ms: u64,
    pub revision: u64,
    pub submitted: u64,
    pub done: bool,
    pub truth: Pose,
    pub historical: Estimate,
    pub current: Estimate,
    pub observations: Vec<Observation>,
}
impl Report {
    pub fn validate(&self) -> Result<(), String> {
        let finite = |p: Pose| p.x.is_finite() && p.y.is_finite() && p.heading.is_finite();
        if self.clock_ms > (u64::MAX - 1) / 1_000_000
            || self.query_ms > self.clock_ms
            || self.observations.len() > 128
            || !finite(self.truth)
            || [&self.historical, &self.current].into_iter().any(|e| {
                !finite(e.pose)
                    || e.trail.len() > 120
                    || e.route.len() > 4
                    || e.route.iter().any(|p| !finite(*p))
                    || e.proposal.len() > 80
                    || e.trail.windows(2).any(|w| w[0].time_ns >= w[1].time_ns)
                    || e.trail.iter().any(|p| p.time_ns.saturating_sub(1) / 1_000_000 > self.query_ms || !finite(p.pose))
            })
            || self.observations.iter().any(|o| {
                !finite(o.pose) || o.due_ms < o.capture_ms || o.arrived_ms.is_some_and(|at| at < o.capture_ms || at > self.clock_ms)
            })
        {
            return Err("Invalid robotics presentation report".into());
        }
        Ok(())
    }
}
