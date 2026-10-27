//! Project the fully validated movement state into a bounded display scene.
use super::{Driver, SECOND};
pub use crate::support::movement_report::Report;
use crate::support::movement_report::{Client, Point};
impl Driver {
    pub fn report(&self) -> Report {
        let world = |p: [i128; 2]| [p[0] as f64 / SECOND as f64, p[1] as f64 / SECOND as f64];
        let center = self.scene.current.samples.last().and_then(|s| s.positions.first()).copied().map(world).unwrap_or([0.0, 0.0]);
        let visible = |p: &[f64; 2]| (p[0] - center[0]).abs() <= 100.0 && (p[1] - center[1]).abs() <= 100.0;
        let mut positions = Vec::new();
        let mut trails = Vec::new();
        let mut limited = false;
        if let Some(latest) = self.scene.current.samples.last() {
            for (id, p) in latest.positions.iter().enumerate() {
                let position = world(*p);
                if visible(&position) {
                    if positions.len() == 2048 {
                        limited = true;
                        break;
                    }
                    positions.push(Client { id: id + 1, position, corrected: self.scene.corrected.contains(&id) });
                }
            }
        }
        // Current points get the preview budget first and render above ghosts.
        for p in self.scene.current.samples.iter().flat_map(|s| &s.positions) {
            let position = world(*p);
            if visible(&position) {
                if trails.len() == 4096 {
                    limited = true;
                    break;
                }
                trails.push(Point { position, corrected: false, opacity: 1.0, current: true });
            }
        }
        for ghost in self.scene.ghosts.iter().rev() {
            let opacity = (1.0
                - (self.now.as_nanos() as u64 + 1).saturating_sub(ghost.created) as f64 / (self.config.fade_ms * 1_000_000) as f64)
                .clamp(0.0, 1.0);
            for p in self.scene.revised_positions(&ghost.state) {
                let position = world(*p);
                if visible(&position) {
                    if trails.len() == 4096 {
                        limited = true;
                        break;
                    }
                    trails.push(Point { position, corrected: true, opacity, current: false });
                }
            }
            if trails.len() == 4096 {
                break;
            }
        }
        Report {
            clients: self.config.clients,
            client_spacing: self.config.client_spacing,
            movement_speed: self.config.movement_speed,
            workers: self.config.workers,
            average_latency_ms: self.config.average_latency_ms,
            jitter_ms: self.config.jitter_radius_ms(),
            submitted: self.submitted,
            in_flight: self.in_flight(),
            late: self.late(),
            corrections: self.scene.corrections,
            updates: self.updates,
            keyboard_inputs: self.keyboard_inputs,
            center,
            positions,
            trails,
            preview_limited: limited,
        }
    }
}
