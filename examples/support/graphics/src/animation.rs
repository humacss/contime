use crate::report::Report;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
pub struct Marker {
    pub position: [f64; 2],
    pub opacity: f64,
    pub correction: f64,
}
#[derive(Default)]
pub struct Animation {
    pub clients: BTreeMap<usize, Marker>,
    initialized: bool,
}
impl Animation {
    pub fn reset(&mut self) {
        self.clients.clear();
        self.initialized = false;
    }
    pub fn update(&mut self, report: &Report, elapsed_ms: f64, reduced_motion: bool) {
        let elapsed = elapsed_ms.max(0.0);
        let amount = if reduced_motion { 1.0 } else { -(-elapsed / 65.0).exp_m1() };
        let mut present = BTreeSet::new();
        for client in &report.positions {
            present.insert(client.id);
            let existed = self.clients.contains_key(&client.id);
            let marker = self.clients.entry(client.id).or_insert(Marker {
                position: client.position,
                opacity: if self.initialized { 0.0 } else { 1.0 },
                correction: 0.0,
            });
            for (value, target) in marker.position.iter_mut().zip(client.position) {
                *value += (target - *value) * amount;
            }
            if existed {
                marker.opacity = (marker.opacity + elapsed / 180.0).min(1.0);
            }
            marker.correction += (f64::from(client.corrected) - marker.correction) * amount;
            if reduced_motion {
                marker.position = client.position;
                marker.opacity = 1.0;
                marker.correction = f64::from(client.corrected);
            }
        }
        self.clients.retain(|id, marker| {
            if !present.contains(id) {
                marker.opacity = if reduced_motion { 0.0 } else { (marker.opacity - elapsed / 180.0).max(0.0) };
            }
            marker.opacity > 0.0 || present.contains(id)
        });
        self.initialized = true;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::report::{Client, Report};
    pub(crate) fn scene(x: f64) -> Report {
        Report {
            clients: 1,
            workers: 1,
            client_spacing: 4,
            movement_speed: 6,
            average_latency_ms: 50,
            jitter_ms: 25,
            submitted: 1,
            in_flight: 0,
            late: 0,
            corrections: 0,
            updates: 1,
            keyboard_inputs: 0,
            center: [x, 0.0],
            arena_half_extent: [48.0, 28.0],
            positions: vec![Client { id: 1, position: [x, 0.0], corrected: false }],
            trails: vec![],
            preview_limited: false,
        }
    }
    #[test]
    fn markers_approach_queries_without_mutating_or_predicting_state() {
        let mut animation = Animation::default();
        animation.update(&scene(0.0), 0.0, false);
        let report = scene(10.0);
        animation.update(&report, 16.0, false);
        assert!((0.0..10.0).contains(&animation.clients[&1].position[0]));
        assert_eq!(report.positions[0].position, [10.0, 0.0]);
        animation.update(&report, 1000.0, false);
        assert!((animation.clients[&1].position[0] - 10.0).abs() < 0.001);
        assert!(animation.clients[&1].position[0] <= 10.0);
    }
    #[test]
    fn smoothing_is_independent_of_display_frequency() {
        let sample = |step| {
            let mut animation = Animation::default();
            animation.update(&scene(0.0), 0.0, false);
            for _ in 0..(96 / step) {
                animation.update(&scene(10.0), f64::from(step), false);
            }
            animation.clients[&1].position[0]
        };
        assert!((sample(8) - sample(16)).abs() < 1e-10);
    }
    #[test]
    fn reduced_motion_and_reset_use_exact_queried_positions() {
        let mut animation = Animation::default();
        animation.update(&scene(0.0), 0.0, false);
        animation.update(&scene(10.0), 16.0, true);
        assert_eq!(animation.clients[&1].position, [10.0, 0.0]);
        animation.reset();
        animation.update(&scene(-10.0), 0.0, false);
        assert_eq!(animation.clients[&1].position, [-10.0, 0.0]);
    }
    #[test]
    fn removed_markers_fade_out_then_release_their_state() {
        let mut animation = Animation::default();
        animation.update(&scene(0.0), 0.0, false);
        let mut empty = scene(0.0);
        empty.positions.clear();
        animation.update(&empty, 50.0, false);
        assert!(animation.clients[&1].opacity > 0.0);
        animation.update(&empty, 200.0, false);
        assert!(animation.clients.is_empty());
    }
}
