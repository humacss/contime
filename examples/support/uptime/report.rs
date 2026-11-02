//! Bounded presentation contract. This module intentionally depends only on serde and std.
use serde::{Deserialize, Serialize};
pub const MAX_SERVICES: usize = 16;
pub const MAX_INTERVALS: usize = 256;
pub const MAX_INPUTS: usize = 128;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Healthy,
    Degraded,
    Down,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interval {
    pub start_ms: u64,
    pub end_ms: u64,
    pub status: Status,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timeline {
    pub status: Status,
    pub uptime_ms: u64,
    pub downtime_ms: u64,
    pub degraded_ms: u64,
    pub intervals: Vec<Interval>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceReport {
    pub id: usize,
    pub name: String,
    pub reconciled: Timeline,
    pub arrival_order: Timeline,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Input {
    pub id: u64,
    pub service: usize,
    pub status: Status,
    pub original_ms: u64,
    pub delivery_ms: u64,
    pub arrived_ms: Option<u64>,
    pub late: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    /// Configured source cadence; older reports may omit presentation metadata.
    #[serde(default)]
    pub cadence_ms: Option<u64>,
    /// Current simulation/delivery clock, independent of the validated query horizon.
    pub clock_ms: u64,
    /// Time of the last validated ConTime query.
    pub now_ms: u64,
    pub window_start_ms: u64,
    pub services: Vec<ServiceReport>,
    pub inputs: Vec<Input>,
    pub submitted: u64,
    pub completed: u64,
    pub updates: u64,
    pub late: u64,
    pub in_flight: usize,
    pub done: bool,
}
impl Report {
    pub fn validate(&self) -> Result<(), String> {
        const MAX_TIME_MS: u64 = (u64::MAX - 1) / 1_000_000;
        if self.clock_ms > MAX_TIME_MS
            || self.now_ms > self.clock_ms
            || self.window_start_ms > self.now_ms
            || self.services.len() > MAX_SERVICES
            || self.inputs.len() > MAX_INPUTS
            || self.completed > self.submitted
        {
            return Err("Invalid uptime report bounds".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for service in &self.services {
            if service.id == 0 || !ids.insert(service.id) || service.name.len() > 128 {
                return Err("Invalid uptime service".into());
            }
            for timeline in [&service.reconciled, &service.arrival_order] {
                if timeline.intervals.len() > MAX_INTERVALS
                    || timeline.uptime_ms.saturating_add(timeline.downtime_ms) > self.now_ms
                    || timeline.degraded_ms > timeline.uptime_ms
                {
                    return Err("Invalid uptime totals".into());
                }
                let mut end = self.window_start_ms;
                for interval in &timeline.intervals {
                    if interval.start_ms < end || interval.end_ms < interval.start_ms || interval.end_ms > self.now_ms {
                        return Err("Invalid uptime interval".into());
                    }
                    end = interval.end_ms;
                }
            }
        }
        for input in &self.inputs {
            if !ids.contains(&input.service)
                || input.original_ms > MAX_TIME_MS
                || input.delivery_ms > MAX_TIME_MS
                || input.delivery_ms < input.original_ms
                || input.arrived_ms.is_some_and(|at| at < input.original_ms || at > self.clock_ms)
                || (input.late && input.arrived_ms.is_none())
            {
                return Err("Invalid uptime input".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn separate_clocks_allow_pending_future_delivery_and_expedited_stop_arrivals() {
        let mut report = Report {
            clock_ms: 2000,
            now_ms: 1000,
            services: vec![ServiceReport {
                id: 1,
                name: "Service 1".into(),
                reconciled: Timeline::default(),
                arrival_order: Timeline::default(),
            }],
            inputs: vec![Input {
                id: 2,
                service: 1,
                status: Status::Down,
                original_ms: 1000,
                delivery_ms: 61000,
                arrived_ms: None,
                late: false,
            }],
            ..Default::default()
        };
        report.validate().unwrap();
        report.inputs[0].arrived_ms = Some(2000);
        report.validate().unwrap();
        report.inputs[0].arrived_ms = Some(2001);
        assert!(report.validate().is_err());
        report.inputs[0].arrived_ms = Some(999);
        assert!(report.validate().is_err());
        report.inputs[0].arrived_ms = None;
        report.clock_ms = 999;
        report.now_ms = 999;
        report.validate().unwrap(); // Pending actions may be previewed before their source tick.
        report.inputs[0].arrived_ms = Some(999);
        assert!(report.validate().is_err());
        report.clock_ms = u64::MAX;
        assert!(report.validate().is_err());
    }
}
