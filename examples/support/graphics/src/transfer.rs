//! Presentation-only handoff tracers: delivered timeline input -> historical action.
use crate::{network, report::Report};
use bevy::prelude::*;
use std::collections::BTreeSet;
const DURATION_MS: f64 = 450.0;
struct Flight {
    id: u64,
    client: usize,
    direction: [f64; 2],
    start: Vec2,
    target: Vec2,
    elapsed: f64,
}
impl Flight {
    fn position(&self, t: f32) -> Vec2 {
        let t = t.clamp(0.0, 1.0);
        let t = t * t * (3.0 - 2.0 * t);
        let bend = if self.client.is_multiple_of(2) { 5.0 } else { -5.0 };
        let control = (self.start + self.target) * 0.5 + Vec2::X * bend;
        self.start * (1.0 - t).powi(2) + control * (2.0 * (1.0 - t) * t) + self.target * t.powi(2)
    }
}
#[derive(Default)]
pub struct Transfers {
    flights: Vec<Flight>,
    seen: BTreeSet<u64>,
}
impl Transfers {
    pub fn reset(&mut self) {
        self.flights.clear();
        self.seen.clear();
    }
    pub fn update(
        &mut self,
        report: &Report,
        tracks: &crate::tracks::Tracks,
        entries: &crate::entrance::Entries,
        elapsed_ms: f64,
        reduced_motion: bool,
    ) {
        // Retain identities only while the bounded arrival preview retains them.
        self.seen.retain(|id| report.arrivals.iter().any(|a| a.id == *id));
        for flight in &mut self.flights {
            flight.elapsed += elapsed_ms.max(0.0);
            if let Some(action) = report.arrivals.iter().find(|a| a.id == flight.id) {
                flight.target = Vec2::from_array(action.position.map(|v| v as f32));
            }
        }
        self.flights.retain(|f| f.elapsed < DURATION_MS);
        if reduced_motion {
            self.flights.clear();
        }
        for event in &report.network {
            if event.arrived_ns.is_none() || self.seen.contains(&event.id) {
                continue;
            }
            // Very short network delays can complete before the entrance animation.
            // Delay only this visual handoff, never the input or queried state.
            if !reduced_motion && !entries.complete(event.id) {
                continue;
            }
            let Some(action) = report.arrivals.iter().find(|a| a.id == event.id) else {
                continue;
            };
            self.seen.insert(event.id);
            // Bound animation work independently of input rate. Old arrivals
            // first seen after a pause or slow render must not play back later.
            if reduced_motion || action.pulse <= 0.0 || self.flights.len() >= 32 {
                continue;
            }
            self.flights.push(Flight {
                id: event.id,
                client: event.client,
                direction: event.direction,
                start: network::delivery_position(report, event, tracks),
                target: Vec2::from_array(action.position.map(|v| v as f32)),
                elapsed: 0.0,
            });
        }
    }
    pub fn draw(&self, gizmos: &mut Gizmos) {
        for flight in &self.flights {
            let t = (flight.elapsed / DURATION_MS) as f32;
            let p = flight.position(t);
            let alpha = (1.0 - t.powi(4)).max(0.0);
            // Keep the wake short regardless of travel distance or flight speed.
            let tail = flight.position((t - 0.025).max(0.0)) - p;
            let tail = tail * (2.0 / tail.length().max(2.0));
            for i in 0..4 {
                let a = p + tail * (i as f32 / 4.0);
                let b = p + tail * ((i + 1) as f32 / 4.0);
                gizmos.line_2d(a, b, crate::palette::client(flight.client, alpha * 0.35 * (1.0 - i as f32 / 4.0)));
            }
            let color = crate::palette::input(flight.client, alpha);
            network::icon(gizmos, p, flight.direction, color, 1.25);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Arrival, NetworkEvent};
    fn delivered() -> Report {
        let mut report = crate::animation::tests::scene(0.0);
        report.now_ns = 100;
        report.network.push(NetworkEvent {
            id: 7,
            client: 1,
            direction: [1.0, 0.0],
            action_ns: 10,
            delivery_ns: 90,
            arrived_ns: Some(100),
            reordered: false,
        });
        report.arrivals.push(Arrival {
            id: 7,
            client: 1,
            time_ns: 10,
            position: [8.0, 9.0],
            direction: [1.0, 0.0],
            opacity: 1.0,
            pulse: 1.0,
        });
        report
    }
    #[test]
    fn short_delay_waits_for_the_visual_entrance_before_launching() {
        let report = delivered();
        let mut entries = crate::entrance::Entries::default();
        let tracks = crate::tracks::Tracks::default();
        let mut transfers = Transfers::default();
        entries.update(&report, 0.0, false);
        transfers.update(&report, &tracks, &entries, 0.0, false);
        assert!(transfers.flights.is_empty());
        entries.update(&report, 400.0, false);
        transfers.update(&report, &tracks, &entries, 0.0, false);
        assert_eq!(transfers.flights.len(), 1);
        transfers.update(&report, &tracks, &entries, 0.0, false);
        assert_eq!(transfers.flights.len(), 1);
    }
    #[test]
    fn transfer_launches_from_the_inputs_assigned_track() {
        let mut report = delivered();
        let mut earlier = report.network[0].clone();
        earlier.id = 6;
        earlier.action_ns = 0;
        report.network.insert(0, earlier);
        let mut tracks = crate::tracks::Tracks::default();
        tracks.update(&report);
        assert_ne!(tracks.offset(7), 0.0);
        let mut transfers = Transfers::default();
        transfers.update(&report, &tracks, &crate::entrance::Entries::default(), 0.0, false);
        assert_eq!(transfers.flights[0].start, network::delivery_position(&report, &report.network[1], &tracks));
    }
    #[test]
    fn delivery_launches_once_and_lands_on_the_historical_action() {
        let report = delivered();
        let mut transfers = Transfers::default();
        transfers.update(&report, &crate::tracks::Tracks::default(), &crate::entrance::Entries::default(), 0.0, false);
        assert_eq!(transfers.flights.len(), 1);
        let flight = &transfers.flights[0];
        assert_eq!(flight.position(0.0), flight.start);
        assert_eq!(flight.position(1.0), Vec2::new(8.0, 9.0));
        transfers.update(&report, &crate::tracks::Tracks::default(), &crate::entrance::Entries::default(), 200.0, false);
        assert_eq!(transfers.flights.len(), 1);
        transfers.update(&report, &crate::tracks::Tracks::default(), &crate::entrance::Entries::default(), 500.0, false);
        assert!(transfers.flights.is_empty());
        transfers.update(&report, &crate::tracks::Tracks::default(), &crate::entrance::Entries::default(), 0.0, false);
        assert!(transfers.flights.is_empty());
    }
    #[test]
    fn waiting_inputs_reduced_motion_and_reset_never_launch_stale_transfers() {
        let mut report = delivered();
        let mut transfers = Transfers::default();
        report.network[0].arrived_ns = None;
        transfers.update(&report, &crate::tracks::Tracks::default(), &crate::entrance::Entries::default(), 0.0, false);
        assert!(transfers.flights.is_empty());
        report.network[0].arrived_ns = Some(100);
        transfers.update(&report, &crate::tracks::Tracks::default(), &crate::entrance::Entries::default(), 0.0, true);
        transfers.update(&report, &crate::tracks::Tracks::default(), &crate::entrance::Entries::default(), 0.0, false);
        assert!(transfers.flights.is_empty());
        transfers.reset();
        transfers.update(&report, &crate::tracks::Tracks::default(), &crate::entrance::Entries::default(), 0.0, false);
        assert_eq!(transfers.flights.len(), 1);
        transfers.update(
            &crate::animation::tests::scene(0.0),
            &crate::tracks::Tracks::default(),
            &crate::entrance::Entries::default(),
            500.0,
            false,
        );
        assert!(transfers.flights.is_empty() && transfers.seen.is_empty());
    }
}
