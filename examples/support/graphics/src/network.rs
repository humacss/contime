//! Simulated network timing on one shared action/arrival clock.
use crate::{entrance::Entries, report::Report};
use bevy::{math::Isometry2d, prelude::*};

struct Axis {
    now: u64,
    past: f64,
    future: f64,
}
impl Axis {
    fn new(now: u64, average_ms: u64, jitter_ms: u64) -> Self {
        let (past, future) = crate::report::timeline_window(average_ms, jitter_ms);
        Self { now, past, future }
    }
    /// Whole-second boundaries slide with the inputs, without numeric labels.
    fn seconds(&self) -> impl Iterator<Item = f32> + '_ {
        let start = self.now.saturating_sub(self.past as u64).div_ceil(1_000_000_000);
        let end = self.now.saturating_add(self.future as u64) / 1_000_000_000;
        // At extreme latency settings, omit divisions too close to distinguish.
        let visible = self.past + self.future <= 88_000_000_000.0;
        (start..=end).take(if visible { 90 } else { 0 }).map(|second| self.x(second * 1_000_000_000))
    }
    fn x(&self, time: u64) -> f32 {
        let relative = (i128::from(time) - i128::from(self.now)) as f64;
        (-44.0 + 88.0 * ((relative + self.past) / (self.past + self.future)).clamp(0.0, 1.0)) as f32
    }
}
fn text(gizmos: &mut Gizmos, x: f32, y: f32, value: &str, size: f32, color: Color) {
    gizmos.text_2d(Isometry2d::from_translation(Vec2::new(x, y)), value, size, Vec2::new(-0.5, 0.0), color);
}
pub(crate) fn icon(gizmos: &mut Gizmos, p: Vec2, direction: [f64; 2], color: Color, scale: f32) {
    let d = Vec2::from_array(direction.map(|v| v as f32));
    if d == Vec2::ZERO {
        gizmos.circle_2d(p, 0.25 * scale, color).resolution(12);
    } else {
        let tip = p + d * (0.55 * scale);
        let shoulder = p - d * (0.05 * scale);
        gizmos.line_2d(p - d * (0.4 * scale), tip, color);
        gizmos.line_2d(shoulder + d.perp() * (0.3 * scale), tip, color);
        gizmos.line_2d(shoulder - d.perp() * (0.3 * scale), tip, color);
    }
}
/// Brief explanatory label; the double tick remains after this text fades.
fn reordered_opacity(event: &crate::report::NetworkEvent, now: u64) -> f32 {
    if !event.reordered {
        return 0.0;
    }
    event.arrived_ns.map_or(0.0, |at| ((1_200_000_000.0 - now.saturating_sub(at) as f32) / 500_000_000.0).clamp(0.0, 1.0))
}
pub(crate) const RULER: f32 = 41.0;
/// Stable rows, top to bottom: player, then the four simulated clients.
fn client_y(client: usize) -> f32 {
    65.0 - client.saturating_sub(1) as f32 * 5.0
}
pub(crate) fn delivery_position(report: &Report, event: &crate::report::NetworkEvent, tracks: &crate::tracks::Tracks) -> Vec2 {
    let axis = Axis::new(report.now_ns, report.average_latency_ms, report.jitter_ms);
    Vec2::new(axis.x(event.arrived_ns.unwrap_or(event.delivery_ns)), client_y(event.client) + tracks.offset(event.id))
}
pub fn draw(report: &Report, entries: &Entries, tracks: &crate::tracks::Tracks, reduced_motion: bool, gizmos: &mut Gizmos) {
    let axis = Axis::new(report.now_ns, report.average_latency_ms, report.jitter_ms);
    let now = axis.x(report.now_ns);
    let muted = Color::srgb(0.50, 0.60, 0.65);
    let bright = Color::srgb(0.78, 0.87, 0.90);
    text(gizmos, -48.0, 72.5, "INPUT TIMELINE", 1.3, bright);
    text(gizmos, now - 3.5, 70.0, "DELIVERY", 1.1, bright);
    gizmos.line_2d(Vec2::new(now, 68.5), Vec2::new(now, RULER - 0.4), bright);
    gizmos.line_2d(Vec2::new(-44.0, RULER), Vec2::new(44.0, RULER), muted);
    for x in axis.seconds() {
        gizmos.line_2d(Vec2::new(x, RULER), Vec2::new(x, 67.0), Color::srgba(0.50, 0.60, 0.65, 0.14));
        gizmos.line_2d(Vec2::new(x, RULER - 0.5), Vec2::new(x, RULER), muted);
    }
    // Every client keeps its own row, even when there are no visible inputs.
    for client in 1..=report.clients {
        let y = client_y(client);
        let swatch = Vec2::new(-47.0, y);
        gizmos.circle_2d(swatch, 0.42, crate::palette::client(client, 1.0)).resolution(16);
        if client == 1 {
            gizmos.circle_2d(swatch, 0.8, bright).resolution(20);
        }
        gizmos.line_2d(Vec2::new(-44.0, y), Vec2::new(44.0, y), Color::srgba(0.50, 0.60, 0.65, 0.12));
    }
    // Draw delay lines first, then action icons on top, on the same client row.
    for event in &report.network {
        let start = axis.x(event.action_ns);
        let y = client_y(event.client) + tracks.offset(event.id);
        let entrance = entries.geometry(
            event.id,
            Vec2::new(-47.0, client_y(event.client)),
            Vec2::new(start, y),
            axis.x(event.arrived_ns.unwrap_or(event.delivery_ns)),
        );
        if entrance.reveal == 0.0 {
            continue;
        }
        let end = entrance.end;
        let arrived = event.arrived_ns.is_some();
        let color = crate::palette::client(event.client, if arrived { 0.80 } else { 0.45 });
        let line_start = (start + 0.9).min(end);
        if line_start < end.min(now) {
            gizmos.line_2d(Vec2::new(line_start, y), Vec2::new(end.min(now), y), color);
        }
        if end > now {
            gizmos.line_2d(Vec2::new(line_start.max(now), y), Vec2::new(end, y), color);
        }
        // A quiet dot preserves the scheduled time on the same horizontal line.
        let due = axis.x(event.delivery_ns);
        if due <= end {
            gizmos.circle_2d(Vec2::new(due, y), 0.12, muted).resolution(8);
        }
        let pulse = if reduced_motion {
            0.0
        } else {
            event.arrived_ns.map_or(0.0, |at| (1.0 - report.now_ns.saturating_sub(at) as f32 / 1_500_000_000.0).clamp(0.0, 1.0))
        };
        let size = (0.4 + 0.3 * pulse) * entrance.reveal;
        let tip = Vec2::new(end, y);
        let delivery_color = crate::palette::input(event.client, 1.0);
        // The action carries direction; its delivery end is just a time boundary.
        if event.reordered {
            for offset in [-0.20, 0.20] {
                let tick = tip + Vec2::X * offset;
                gizmos.line_2d(tick - Vec2::Y * size, tick + Vec2::Y * size, delivery_color);
            }
            let alpha = reordered_opacity(event, report.now_ns);
            if alpha > 0.0 && entries.complete(event.id) {
                text(gizmos, (end + 0.7).min(35.0), y + 1.0, "reordered", 0.9, delivery_color.with_alpha(alpha));
            }
        } else {
            gizmos.line_2d(tip - Vec2::Y * size, tip + Vec2::Y * size, delivery_color);
        }
    }
    for event in &report.network {
        // Clip old action icons instead of piling them up at the left boundary;
        // their delay lines may still have a visible delivery end.
        if report.now_ns.saturating_sub(event.action_ns) as f64 > axis.past {
            continue;
        }
        let action = Vec2::new(axis.x(event.action_ns), client_y(event.client) + tracks.offset(event.id));
        let p = entries
            .geometry(event.id, Vec2::new(-47.0, client_y(event.client)), action, axis.x(event.arrived_ns.unwrap_or(event.delivery_ns)))
            .position;
        let color = crate::palette::input(event.client, 1.0);
        let (scale, alpha) = entries.appearance(event.id);
        let color = color.with_alpha(alpha);
        icon(gizmos, p, event.direction, color, scale * tracks.icon_scale(event.id));
    }
    if report.network.is_empty() {
        text(gizmos, -44.0, 58.0, "No inputs waiting or recently delivered", 1.2, muted);
    }
    text(gizmos, -48.0, 36.0, "Arrow: direction / Dot: Stop / Line: network delay", 1.0, muted);
    text(gizmos, -48.0, 33.5, "Tick: delivery time / Double tick: out-of-order arrival", 1.0, muted);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reordered_label_requires_an_arrival_and_fades_without_changing_identity() {
        let mut event = crate::report::NetworkEvent {
            id: 1,
            client: 2,
            direction: [1.0, 0.0],
            action_ns: 0,
            delivery_ns: 1,
            arrived_ns: None,
            reordered: true,
        };
        assert_eq!(reordered_opacity(&event, 1), 0.0);
        event.arrived_ns = Some(1);
        assert_eq!(reordered_opacity(&event, 1), 1.0);
        assert!((reordered_opacity(&event, 950_000_001) - 0.5).abs() < 0.001);
        assert_eq!(reordered_opacity(&event, 1_200_000_001), 0.0);
        event.reordered = false;
        assert_eq!(reordered_opacity(&event, 1), 0.0);
    }
    #[test]
    fn timeline_divisions_represent_whole_seconds() {
        let axis = Axis::new(12_300_000_000, 600, 600);
        let divisions: Vec<_> = axis.seconds().collect();
        assert_eq!(divisions, [9, 10, 11, 12, 13, 14].map(|s| axis.x(s * 1_000_000_000)));
        assert_eq!(axis.past + axis.future, 6_000_000_000.0);
        assert!((axis.x(axis.now) - (-44.0 + 88.0 * 4.0 / 7.0)).abs() < 0.0001);
        let second_width = 88.0 / 6.0;
        for pair in divisions.windows(2) {
            assert!((pair[1] - pair[0] - second_width).abs() < 0.0001);
        }
    }
    #[test]
    fn timeline_and_transfer_origins_are_above_the_arena() {
        let report = crate::animation::tests::scene(0.0);
        let event = crate::report::NetworkEvent {
            id: 1,
            client: 1,
            direction: [1.0, 0.0],
            action_ns: 0,
            delivery_ns: 1,
            arrived_ns: Some(1),
            reordered: false,
        };
        assert!(RULER > report.arena_half_extent[1] as f32 + 2.0);
        assert!(delivery_position(&report, &event, &crate::tracks::Tracks::default()).y > RULER);
    }
    #[test]
    fn client_rows_are_stable_for_actions_and_delivery_origins() {
        let mut report = crate::animation::tests::scene(0.0);
        let mut event = crate::report::NetworkEvent {
            id: 1,
            client: 2,
            direction: [1.0, 0.0],
            action_ns: 0,
            delivery_ns: 10,
            arrived_ns: Some(10),
            reordered: false,
        };
        let origin = delivery_position(&report, &event, &crate::tracks::Tracks::default());
        assert_eq!(origin.y, client_y(2));
        report.network.push(event.clone());
        event.id = 2;
        event.action_ns = 20;
        event.delivery_ns = 30;
        event.arrived_ns = None;
        report.network.insert(0, event.clone());
        assert_eq!(delivery_position(&report, &event, &crate::tracks::Tracks::default()).y, origin.y);
        for client in 1..=crate::report::MAX_CLIENTS {
            event.client = client;
            assert_eq!(delivery_position(&report, &event, &crate::tracks::Tracks::default()).y, client_y(client));
            assert!(client_y(client) > RULER);
            if client > 1 {
                assert_eq!(client_y(client - 1) - client_y(client), 5.0);
            }
        }
    }
    #[test]
    fn network_reports_reject_invalid_times_and_oversized_previews() {
        let mut report = crate::animation::tests::scene(0.0);
        report.now_ns = 100;
        let event = crate::report::NetworkEvent {
            id: 1,
            client: 1,
            direction: [1.0, 0.0],
            action_ns: 20,
            delivery_ns: 80,
            arrived_ns: Some(90),
            reordered: false,
        };
        report.network.push(event.clone());
        assert!(report.validate().is_ok());
        report.network[0].arrived_ns = Some(70);
        assert!(report.validate().is_err());
        report.network[0] = event.clone();
        report.network[0].action_ns = 101;
        assert!(report.validate().is_err());
        report.network = vec![event; crate::report::MAX_NETWORK_EVENTS + 1];
        assert!(report.validate().is_err());
    }
    #[test]
    fn action_and_delivery_share_a_clock_and_can_reverse_order() {
        let axis = Axis::new(12_300_000_000, 600, 600);
        assert!(axis.x(12_000_000_000) < axis.x(12_200_000_000));
        assert!(axis.x(12_900_000_000) > axis.x(12_450_000_000));
        assert!(axis.x(12_200_000_000) < axis.x(axis.now));
        assert!(axis.x(12_450_000_000) > axis.x(axis.now));
        assert!(axis.x(0).is_finite());
        assert!(axis.x(u64::MAX).is_finite());
    }
}
