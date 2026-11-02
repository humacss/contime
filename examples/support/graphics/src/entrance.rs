//! Presentation-only entrance: client marker, action position, then delay line.
use crate::report::Report;
use bevy::prelude::Vec2;

struct Entry {
    elapsed_ms: f64,
    duration_ms: f64,
}
impl Entry {
    fn complete() -> Self {
        Self { elapsed_ms: 400.0, duration_ms: 400.0 }
    }
    fn age(&self) -> f64 {
        (self.elapsed_ms / self.duration_ms).clamp(0.0, 1.0) * 400.0
    }
}
/// Animate first visibility within the remaining network delay, never after delivery.
#[derive(Default)]
pub(crate) struct Entries(std::collections::BTreeMap<u64, Entry>);
impl Entries {
    pub fn reset(&mut self) {
        self.0.clear();
    }
    pub fn update(&mut self, report: &Report, elapsed_ms: f64, reduced_motion: bool) {
        self.0.retain(|id, _| report.network.iter().any(|event| event.id == *id) || report.arrivals.iter().any(|event| event.id == *id));
        for entry in self.0.values_mut() {
            entry.elapsed_ms = (entry.elapsed_ms + elapsed_ms.max(0.0)).min(entry.duration_ms);
        }
        for event in &report.network {
            let remaining_ms = event.delivery_ns.saturating_sub(report.now_ns) as f64 / 1_000_000.0;
            if reduced_motion || event.arrived_ns.is_some() || remaining_ms == 0.0 {
                self.0.insert(event.id, Entry::complete());
                continue;
            }
            let entry = self.0.entry(event.id).or_insert(Entry { elapsed_ms: 0.0, duration_ms: remaining_ms.min(400.0) });
            // Host reports can advance farther than the rendered frame delta.
            // Catch up to the delivery deadline without depending on query progress.
            entry.elapsed_ms = entry.elapsed_ms.max(entry.duration_ms - remaining_ms).min(entry.duration_ms);
        }
        if reduced_motion {
            self.0.values_mut().for_each(|entry| *entry = Entry::complete());
        }
    }
    pub fn complete(&self, id: u64) -> bool {
        self.0.get(&id).map_or(400.0, Entry::age) >= 400.0
    }
    pub fn geometry(&self, id: u64, origin: Vec2, action: Vec2, delivery_x: f32) -> Entrance {
        let age = self.0.get(&id).map_or(400.0, Entry::age) as f32;
        let travel = (age / 240.0).clamp(0.0, 1.0);
        let travel = 1.0 - (1.0 - travel).powi(3);
        let reveal = ((age - 240.0) / 160.0).clamp(0.0, 1.0);
        let reveal = reveal * reveal * (3.0 - 2.0 * reveal);
        Entrance { position: origin.lerp(action, travel), end: action.x + (delivery_x - action.x) * reveal, reveal }
    }
    pub fn appearance(&self, id: u64) -> (f32, f32) {
        let t = (self.0.get(&id).map_or(400.0, Entry::age) / 400.0).clamp(0.0, 1.0) as f32;
        (1.0 + 0.9 * (1.0 - t).powi(3), 0.25 + 0.75 * (t * 3.0).min(1.0))
    }
}

pub(crate) struct Entrance {
    pub position: Vec2,
    pub end: f32,
    pub reveal: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn short_delay_entrance_finishes_by_delivery_without_waiting_for_a_query() {
        let mut report = crate::animation::tests::scene(0.0);
        report.now_ns = 1_000_000_000;
        report.network.push(crate::report::NetworkEvent {
            id: 1,
            client: 2,
            direction: [1.0, 0.0],
            action_ns: report.now_ns,
            delivery_ns: report.now_ns + 50_000_000,
            arrived_ns: None,
            reordered: false,
        });
        let mut entries = Entries::default();
        entries.update(&report, 0.0, false);
        assert!(!entries.complete(1));
        report.now_ns += 50_000_000;
        entries.update(&report, 50.0, false);
        assert!(entries.complete(1), "a 50 ms delivery cannot wait for a 400 ms entrance");
    }
    #[test]
    fn first_seen_arrived_input_is_already_placed_in_the_timeline() {
        let mut report = crate::animation::tests::scene(0.0);
        report.now_ns = 2_000_000_000;
        report.network.push(crate::report::NetworkEvent {
            id: 1,
            client: 2,
            direction: [1.0, 0.0],
            action_ns: 1_000_000_000,
            delivery_ns: 1_100_000_000,
            arrived_ns: Some(1_110_000_000),
            reordered: false,
        });
        let mut entries = Entries::default();
        entries.update(&report, 0.0, false);
        assert!(entries.complete(1));
        assert_eq!(entries.appearance(1), (1.0, 1.0));
    }
    #[test]
    fn entrance_moves_from_client_marker_then_grows_the_actual_delay_line() {
        let mut entries = Entries::default();
        let origin = Vec2::new(-47.0, 60.0);
        let action = Vec2::new(5.0, 61.4);
        entries.0.insert(1, Entry { elapsed_ms: 0.0, duration_ms: 400.0 });
        let start = entries.geometry(1, origin, action, 20.0);
        assert_eq!(start.position, origin);
        assert_eq!(start.reveal, 0.0);
        entries.0.insert(1, Entry { elapsed_ms: 120.0, duration_ms: 400.0 });
        let moving = entries.geometry(1, origin, action, 20.0);
        assert!(moving.position.x > origin.x && moving.position.x < action.x);
        assert_eq!(moving.reveal, 0.0);
        entries.0.insert(1, Entry { elapsed_ms: 240.0, duration_ms: 400.0 });
        assert_eq!(entries.geometry(1, origin, action, 20.0).position, action);
        entries.0.insert(1, Entry { elapsed_ms: 320.0, duration_ms: 400.0 });
        let growing = entries.geometry(1, origin, action, 20.0);
        assert_eq!(growing.position, action);
        assert!(growing.end > action.x && growing.end < 20.0);
        assert!(!entries.complete(1));
        entries.0.insert(1, Entry { elapsed_ms: 400.0, duration_ms: 400.0 });
        let complete = entries.geometry(1, origin, action, 20.0);
        assert_eq!((complete.position, complete.end, complete.reveal), (action, 20.0, 1.0));
        assert!(entries.complete(1));
        assert_eq!(entries.geometry(1, origin, action, action.x).end, action.x, "zero delay remains zero");
    }
    #[test]
    fn entry_pop_starts_on_visibility_freezes_and_does_not_repeat_on_delivery() {
        let mut report = crate::animation::tests::scene(0.0);
        report.network.push(crate::report::NetworkEvent {
            id: 1,
            client: 1,
            direction: [1.0, 0.0],
            action_ns: 0,
            delivery_ns: 20_000_000_000,
            arrived_ns: None,
            reordered: false,
        });
        report.now_ns = 10_000_000_000; // An old action newly entering the bounded preview.
        let mut entries = Entries::default();
        entries.update(&report, 16.0, false);
        assert_eq!(entries.appearance(1), (1.9, 0.25));
        entries.update(&report, 200.0, false);
        let halfway = entries.appearance(1);
        assert!(halfway.0 > 1.0 && halfway.0 < 1.9);
        entries.update(&report, 0.0, false);
        assert_eq!(entries.appearance(1), halfway);
        report.network[0].arrived_ns = Some(1);
        entries.update(&report, 200.0, false);
        assert_eq!(entries.appearance(1), (1.0, 1.0));
        entries.update(&report, 16.0, false);
        assert_eq!(entries.appearance(1), (1.0, 1.0));
        entries.reset();
        entries.update(&report, 0.0, false);
        assert_eq!(entries.appearance(1), (1.0, 1.0), "reset must not replay an already delivered entrance");
        entries.update(&report, 0.0, true);
        assert_eq!(entries.appearance(1), (1.0, 1.0));
        report.network.clear();
        entries.update(&report, 16.0, false);
        assert!(entries.0.is_empty());
    }
}
