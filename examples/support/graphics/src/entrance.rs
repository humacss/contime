//! Presentation-only entrance: client marker, action position, then delay line.
use crate::report::Report;
use bevy::prelude::Vec2;

/// Entry animation follows first visibility, independently of action or delivery time.
#[derive(Default)]
pub(crate) struct Entries(std::collections::BTreeMap<u64, f64>);
impl Entries {
    pub fn reset(&mut self) {
        self.0.clear();
    }
    pub fn update(&mut self, report: &Report, elapsed_ms: f64, reduced_motion: bool) {
        self.0.retain(|id, _| report.network.iter().any(|event| event.id == *id) || report.arrivals.iter().any(|event| event.id == *id));
        for age in self.0.values_mut() {
            *age = (*age + elapsed_ms.max(0.0)).min(400.0);
        }
        for event in &report.network {
            self.0.entry(event.id).or_insert(0.0);
        }
        if reduced_motion {
            self.0.values_mut().for_each(|age| *age = 400.0);
        }
    }
    pub fn complete(&self, id: u64) -> bool {
        self.0.get(&id).copied().unwrap_or(400.0) >= 400.0
    }
    pub fn geometry(&self, id: u64, origin: Vec2, action: Vec2, delivery_x: f32) -> Entrance {
        let age = self.0.get(&id).copied().unwrap_or(400.0) as f32;
        let travel = (age / 240.0).clamp(0.0, 1.0);
        let travel = 1.0 - (1.0 - travel).powi(3);
        let reveal = ((age - 240.0) / 160.0).clamp(0.0, 1.0);
        let reveal = reveal * reveal * (3.0 - 2.0 * reveal);
        Entrance { position: origin.lerp(action, travel), end: action.x + (delivery_x - action.x) * reveal, reveal }
    }
    pub fn appearance(&self, id: u64) -> (f32, f32) {
        let t = (self.0.get(&id).copied().unwrap_or(400.0) / 400.0).clamp(0.0, 1.0) as f32;
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
    fn entrance_moves_from_client_marker_then_grows_the_actual_delay_line() {
        let mut entries = Entries::default();
        let origin = Vec2::new(-47.0, 60.0);
        let action = Vec2::new(5.0, 61.4);
        entries.0.insert(1, 0.0);
        let start = entries.geometry(1, origin, action, 20.0);
        assert_eq!(start.position, origin);
        assert_eq!(start.reveal, 0.0);
        entries.0.insert(1, 120.0);
        let moving = entries.geometry(1, origin, action, 20.0);
        assert!(moving.position.x > origin.x && moving.position.x < action.x);
        assert_eq!(moving.reveal, 0.0);
        entries.0.insert(1, 240.0);
        assert_eq!(entries.geometry(1, origin, action, 20.0).position, action);
        entries.0.insert(1, 320.0);
        let growing = entries.geometry(1, origin, action, 20.0);
        assert_eq!(growing.position, action);
        assert!(growing.end > action.x && growing.end < 20.0);
        assert!(!entries.complete(1));
        entries.0.insert(1, 400.0);
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
            delivery_ns: 1,
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
        assert_eq!(entries.appearance(1), (1.9, 0.25));
        entries.update(&report, 0.0, true);
        assert_eq!(entries.appearance(1), (1.0, 1.0));
        report.network.clear();
        entries.update(&report, 16.0, false);
        assert!(entries.0.is_empty());
    }
}
