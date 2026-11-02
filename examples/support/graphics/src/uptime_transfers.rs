//! Bounded, presentation-only delivery handoffs. Never delays workload processing.
use super::{Input, NetworkLayout, Report, Status, history_axis, identity, row, symbol};
use bevy::prelude::*;
use std::collections::BTreeSet;
pub(super) const DURATION_MS: f64 = 450.0;
struct Flight {
    start: Vec2,
    target: Vec2,
    status: Status,
    color: Color,
    age: f64,
}
impl Flight {
    fn position(&self) -> Vec2 {
        let t = (self.age / DURATION_MS).clamp(0.0, 1.0) as f32;
        self.start.lerp(self.target, t * t * (3.0 - 2.0 * t))
    }
}
#[derive(Default)]
pub(crate) struct Transfers {
    seen: BTreeSet<u64>,
    flights: Vec<Flight>,
}
fn destinations(report: &Report, input: &Input) -> Option<[Vec2; 2]> {
    let index = report.services.iter().position(|service| service.id == input.service)?;
    let arrived = input.arrived_ms?;
    let axis = history_axis(report);
    // Wait for queried history to include the handoff, rather than marking its edge.
    if arrived > report.now_ms || input.original_ms < report.window_start_ms {
        return None;
    }
    let row = row(index, report.services.len());
    Some([Vec2::new(axis.x(input.original_ms), row.reconciled), Vec2::new(axis.x(arrived), row.arrival)])
}
impl Transfers {
    pub(crate) fn update(&mut self, report: &Report, layout: &NetworkLayout, elapsed_ms: f64, immediate: bool) {
        self.seen.retain(|id| report.inputs.iter().any(|input| input.id == *id));
        for flight in &mut self.flights {
            flight.age += elapsed_ms.max(0.0);
        }
        self.flights.retain(|flight| flight.age < DURATION_MS);
        if immediate {
            self.flights.clear();
        }
        for (i, input) in report.inputs.iter().enumerate() {
            if self.seen.contains(&input.id) {
                continue;
            }
            let Some(targets) = destinations(report, input) else { continue };
            self.seen.insert(input.id);
            if immediate || report.clock_ms.saturating_sub(input.arrived_ms.unwrap()) > 1_000 || self.flights.len() >= 64 {
                continue;
            }
            let start = layout.positions[i];
            for target in targets {
                self.flights.push(Flight { start, target, status: input.status, color: identity(input.service, 0.85), age: 0.0 });
            }
        }
    }
    #[cfg(test)]
    pub(super) fn flight_count(&self) -> usize {
        self.flights.len()
    }
    pub(crate) fn draw(&self, gizmos: &mut Gizmos) {
        for flight in &self.flights {
            symbol(gizmos, flight.position(), flight.status, 1.1, flight.color);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::uptime_report::ServiceReport;
    fn report() -> Report {
        Report {
            clock_ms: 1_000,
            now_ms: 1_000,
            services: vec![ServiceReport {
                id: 1,
                name: "Service 1".into(),
                reconciled: Default::default(),
                arrival_order: Default::default(),
            }],
            inputs: vec![Input {
                id: 1,
                service: 1,
                status: Status::Down,
                original_ms: 200,
                delivery_ms: 800,
                arrived_ms: Some(800),
                late: true,
            }],
            ..Default::default()
        }
    }
    #[test]
    fn same_input_flies_to_original_and_arrival_times_together_only_once() {
        let report = report();
        let mut transfers = Transfers::default();
        transfers.update(&report, &NetworkLayout::new(&report), 0.0, false);
        assert_eq!(transfers.flights.len(), 2);
        let a = &transfers.flights[0];
        let b = &transfers.flights[1];
        assert_eq!(a.start, b.start);
        assert_eq!(a.start, NetworkLayout::new(&report).positions[0]);
        assert!(a.target.x < b.target.x);
        assert!(a.target.y > b.target.y);
        transfers.update(&report, &NetworkLayout::new(&report), 225.0, false);
        assert_eq!(transfers.flights.len(), 2);
        assert_eq!(transfers.flights[0].age, transfers.flights[1].age);
        let paused = transfers.flights[0].position();
        transfers.update(&report, &NetworkLayout::new(&report), 0.0, false);
        assert_eq!(transfers.flights[0].position(), paused);
        transfers.update(&report, &NetworkLayout::new(&report), 225.0, false);
        assert!(transfers.flights.is_empty());
    }
    #[test]
    fn waits_for_queried_history_and_skips_stale_or_reduced_motion_handoffs() {
        let mut report = report();
        let mut transfers = Transfers::default();
        report.now_ms = 700;
        transfers.update(&report, &NetworkLayout::new(&report), 0.0, false);
        assert!(transfers.flights.is_empty());
        report.now_ms = 1_000;
        transfers.update(&report, &NetworkLayout::new(&report), 0.0, false);
        assert_eq!(transfers.flights.len(), 2);
        transfers.update(&report, &NetworkLayout::new(&report), 0.0, true);
        assert!(transfers.flights.is_empty());
        report.clock_ms = 3_000;
        let mut fresh = Transfers::default();
        fresh.update(&report, &NetworkLayout::new(&report), 0.0, false);
        assert!(fresh.flights.is_empty());
    }
}
