//! Project the fully validated movement state into a bounded display scene.
use super::{Driver, SECOND};
pub use crate::support::movement_report::Report;
use crate::support::movement_report::{Arrival, Client, Point};
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
                    positions.push(Client {
                        id: id + 1,
                        position,
                        direction: self.scene.current.directions.get(id).copied().unwrap_or([0.0, 0.0]),
                        corrected: self.scene.corrected.contains(&id),
                    });
                }
            }
        }
        // Preserve client/time boundaries; draw current segments over revisions.
        let oldest = self.scene.current.samples.first().map_or(0, |s| s.time.0);
        let now = self.now.as_nanos() as u64 + 1;
        let brightness = |time: u64| {
            0.18 + 0.82 * (1.0 - now.saturating_sub(time) as f64 / (self.config.trail_ms * 1_000_000).max(1) as f64).clamp(0.0, 1.0)
        };
        let mut seen_revisions = std::collections::BTreeSet::new();
        let mut append = |state: &super::render_state::Rendering, current: bool, fade: f64| {
            for (client, time_ns, from, to) in self.scene.segments(state, !current) {
                let previous = world(from);
                let position = world(to);
                if !visible(&position) && !visible(&previous) {
                    continue;
                }
                // Newest copies are visited first. Drawing the same revised
                // geometry repeatedly would brighten it and exhaust the preview.
                if !current && !seen_revisions.insert((client, time_ns, from, to)) {
                    continue;
                }
                if trails.len() == 4096 {
                    limited = true;
                    break;
                }
                trails.push(Point {
                    previous: Some(previous),
                    client,
                    time_ns,
                    position,
                    corrected: !current,
                    opacity: fade * brightness(time_ns),
                    current,
                });
            }
        };
        append(&self.scene.current, true, 1.0);
        for ghost in self.scene.ghosts.iter().rev() {
            let opacity = (1.0
                - (self.now.as_nanos() as u64 + 1).saturating_sub(ghost.created) as f64 / (self.config.fade_ms * 1_000_000) as f64)
                .clamp(0.0, 1.0);
            append(&ghost.state, false, opacity);
        }
        let arrivals = self
            .arrivals
            .iter()
            // Delivery is visible immediately, even while a slower query is pending.
            .filter(|(input, _)| input.time.0 >= oldest)
            .map(|(input, arrived)| Arrival {
                id: input.id as u64,
                client: input.client_id as usize,
                time_ns: input.time.0,
                position: world(self.reference.position(input.client_id as usize, input.time)),
                direction: input.direction.velocity(1).map(|v| v as f64),
                opacity: brightness(input.time.0),
                pulse: (1.0 - (self.now.as_nanos() as u64 + 1).saturating_sub(*arrived) as f64 / 800_000_000.0).clamp(0.0, 1.0),
            })
            .collect();
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
            arena_half_extent: crate::movement_example::model::ARENA_HALF_EXTENT.map(|value| value as f64 / SECOND as f64),
            positions,
            trails,
            arrivals,
            now_ns: now,
            network: self.delivery.timeline(
                now,
                crate::support::movement_report::timeline_window(self.config.average_latency_ms, self.config.jitter_ms).0 as u64,
            ),
            preview_limited: limited,
        }
    }
}
