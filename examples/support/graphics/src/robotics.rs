//! Presentation only. Estimates and proposals come from validated workload reports.
use crate::{
    labels::{Labels, Style},
    robotics_report::{Estimate, Pose, Report},
};
use bevy::prelude::*;
use std::collections::{BTreeSet, VecDeque};
const TEXT: Color = Color::srgb(0.82, 0.90, 0.94);
const MUTED: Color = Color::srgb(0.51, 0.63, 0.70);
const LEFT: Color = Color::srgb(0.35, 0.86, 0.88);
const RIGHT: Color = Color::srgb(0.98, 0.65, 0.48);
const CAMERA: Color = Color::srgb(0.75, 0.65, 0.96);
const FLIGHT_MS: f64 = 450.0;
struct Flight {
    capture: u64,
    arrived: u64,
    pose: Pose,
    start: f64,
}
struct Ghost {
    left: Estimate,
    right: Estimate,
    start: f64,
}
#[derive(Default)]
pub(crate) struct Display {
    clock: f64,
    revision: Option<u64>,
    visible: Report,
    pending: VecDeque<(f64, Report)>,
    ghosts: VecDeque<Ghost>,
    seen: BTreeSet<u64>,
    flights: Vec<Flight>,
    poses: [Pose; 2],
    truth: Pose,
    initialized: bool,
}
fn axis(time: u64, now: f64) -> f32 {
    (-96.0 + (time as f64 - now + 6500.0) / 10000.0 * 192.0).clamp(-96.0, 96.0) as f32
}
fn point(p: Pose, offset: f32) -> Vec2 {
    Vec2::new(offset - 32.0 + p.x as f32 * 5.3, -31.0 + p.y as f32 * 5.3)
}
fn blend(current: &mut Pose, target: Pose, weight: f64) {
    current.x += (target.x - current.x) * weight;
    current.y += (target.y - current.y) * weight;
    let delta = (target.heading - current.heading + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI;
    current.heading += delta * weight;
}
fn revised(old: &Estimate, new: &Estimate) -> bool {
    old.trail.iter().any(|p| {
        new.trail.iter().find(|q| q.time_ns == p.time_ns).is_some_and(|q| (p.pose.x - q.pose.x).abs() + (p.pose.y - q.pose.y).abs() > 0.01)
    })
}
impl Display {
    fn observe(&mut self, report: &Report, immediate: bool) {
        self.seen.retain(|id| report.observations.iter().any(|o| o.id == *id));
        if immediate || self.pending.len() >= 64 {
            self.visible = report.clone();
            self.pending.clear();
            self.flights.clear();
            self.ghosts.clear();
            self.revision = Some(report.revision);
            self.seen.extend(report.observations.iter().filter(|o| o.arrived_ms.is_some()).map(|o| o.id));
            return;
        }
        for observation in &report.observations {
            if let Some(arrived) = observation.arrived_ms
                && arrived <= report.query_ms
                && self.seen.insert(observation.id)
                && report.clock_ms.saturating_sub(arrived) < 1500
            {
                self.flights.push(Flight { capture: observation.capture_ms, arrived, pose: observation.pose, start: self.clock });
            }
        }
        if self.revision != Some(report.revision) {
            self.revision = Some(report.revision);
            self.pending.push_back((self.clock + FLIGHT_MS, report.clone()));
        }
        while self.pending.front().is_some_and(|(at, _)| *at <= self.clock) {
            let next = self.pending.pop_front().unwrap().1;
            if revised(&self.visible.historical, &next.historical) || revised(&self.visible.current, &next.current) {
                self.ghosts.push_back(Ghost {
                    left: self.visible.historical.clone(),
                    right: self.visible.current.clone(),
                    start: self.clock,
                });
                while self.ghosts.len() > 4 {
                    self.ghosts.pop_front();
                }
            }
            self.visible = next;
        }
        self.flights.retain(|flight| self.clock - flight.start < FLIGHT_MS);
        self.ghosts.retain(|ghost| self.clock - ghost.start < 2500.0);
    }
    pub fn draw(&mut self, report: &Report, elapsed: f64, immediate: bool, labels: &mut Labels, gizmos: &mut Gizmos) {
        self.clock = (self.clock + elapsed.max(0.0)).max(report.clock_ms as f64).min(report.clock_ms as f64 + 100.0);
        self.observe(report, immediate);
        let weight = if immediate || !self.initialized { 1.0 } else { 1.0 - (-elapsed.max(0.0) / 120.0).exp() };
        blend(&mut self.poses[0], self.visible.historical.pose, weight);
        blend(&mut self.poses[1], self.visible.current.pose, weight);
        blend(&mut self.truth, self.visible.truth, weight);
        self.initialized = true;
        labels.push(Vec2::new(-100.0, 85.0), 200.0, "SENSOR CAPTURE / RESULT AVAILABLE", Style::Heading, TEXT);
        let now = axis(self.clock as u64, self.clock);
        labels.push(Vec2::new(now - 4.0, 78.5), 45.0, "RESULT AVAILABLE", Style::Legend, TEXT);
        gizmos.line_2d(Vec2::new(now, 47.0), Vec2::new(now, 76.0), MUTED);
        labels.push(Vec2::new(-104.0, 73.0), 22.0, "Wheels", Style::Status, MUTED);
        labels.push(Vec2::new(-104.0, 62.0), 22.0, "Camera", Style::Status, CAMERA);
        let first = (self.clock as u64).saturating_sub(6500).div_ceil(100);
        for tick in first..=first + 100 {
            let time = tick * 100;
            let x = axis(time, self.clock);
            gizmos.line_2d(Vec2::new(x, 70.0), Vec2::new(x, if tick.is_multiple_of(10) { 74.0 } else { 71.4 }), MUTED);
            if tick.is_multiple_of(10) {
                gizmos.line_2d(Vec2::new(x, 47.0), Vec2::new(x, 76.0), Color::srgba(0.5, 0.65, 0.7, 0.12));
            }
        }
        for observation in &report.observations {
            let end = observation.arrived_ms.unwrap_or(observation.due_ms);
            if end as f64 + 6500.0 < self.clock || observation.capture_ms as f64 > self.clock + 3500.0 {
                continue;
            }
            let y = if observation.capture_ms.is_multiple_of(4000) { 62.0 } else { 53.0 };
            let a = axis(observation.capture_ms, self.clock);
            let b = axis(end, self.clock);
            gizmos.line_2d(Vec2::new(a, y), Vec2::new(b, y), CAMERA);
            if observation.capture_ms as f64 + 6500.0 >= self.clock {
                camera(gizmos, Vec2::new(a, y), CAMERA, 0.75);
            }
            if end as f64 <= self.clock + 3500.0 {
                gizmos.line_2d(Vec2::new(b, y - 0.9), Vec2::new(b, y + 0.9), CAMERA);
            }
        }
        labels.push(
            Vec2::new(-100.0, 44.0),
            200.0,
            "Camera icon: capture time · Line: processing delay · End mark: result available",
            Style::Legend,
            MUTED,
        );
        for (index, offset, title, color, estimate) in [
            (0, -53.0, "FROM OBSERVATION TIME", LEFT, &self.visible.historical),
            (1, 53.0, "FROM CURRENT TIME", RIGHT, &self.visible.current),
        ] {
            labels.push(Vec2::new(offset - 48.0, 35.0), 100.0, title, Style::Caption, color);
            labels.push(Vec2::new(offset - 48.0, 28.0), 100.0, &estimate.proposal, Style::Legend, TEXT);
            warehouse(gizmos, offset);
            for ghost in &self.ghosts {
                let old = if index == 0 { &ghost.left } else { &ghost.right };
                let opacity = (1.0 - (self.clock - ghost.start) / 2500.0) as f32 * 0.25;
                trail(gizmos, old, offset, color.with_alpha(opacity));
                route(gizmos, &old.route, offset, color.with_alpha(opacity));
            }
            trail(gizmos, estimate, offset, color.with_alpha(0.75));
            route(gizmos, &estimate.route, offset, color.with_alpha(0.6));
            robot(gizmos, self.truth, offset, MUTED.with_alpha(0.85), true);
            robot(gizmos, self.poses[index], offset, color, false);
            let error = ((estimate.pose.x - self.visible.truth.x).powi(2) + (estimate.pose.y - self.visible.truth.y).powi(2)).sqrt();
            labels.push(
                Vec2::new(offset - 48.0, -46.0),
                100.0,
                &format!("Position error {error:.2} m · {} camera results", estimate.observations),
                Style::Legend,
                TEXT,
            );
        }
        for flight in &self.flights {
            let t = ((self.clock - flight.start) / FLIGHT_MS).clamp(0.0, 1.0) as f32;
            let y = if flight.capture.is_multiple_of(4000) { 62.0 } else { 53.0 };
            for offset in [-53.0, 53.0] {
                let from = Vec2::new(axis(flight.arrived, self.clock), y);
                camera(gizmos, from.lerp(point(flight.pose, offset), t * t * (3.0 - 2.0 * t)), CAMERA, 1.0);
            }
        }
        labels.push(
            Vec2::new(-100.0, -54.0),
            205.0,
            "Gray outline: physical robot · Color: estimate · Dashed: proposed route · Ring: goal",
            Style::Legend,
            MUTED,
        );
    }
}
fn camera(gizmos: &mut Gizmos, p: Vec2, color: Color, size: f32) {
    gizmos.rect_2d(Isometry2d::from_translation(p), Vec2::new(size * 2.0, size * 1.5), color);
    gizmos.circle_2d(p, size * 0.4, color).resolution(12);
}
fn robot(gizmos: &mut Gizmos, pose: Pose, offset: f32, color: Color, outline: bool) {
    let p = point(pose, offset);
    let d = Vec2::new(pose.heading.cos() as f32, pose.heading.sin() as f32);
    if outline {
        gizmos.rect_2d(Isometry2d::new(p, Rot2::radians(pose.heading as f32)), Vec2::new(3.5, 2.4), color);
    } else {
        gizmos.circle_2d(p, 1.3, color).resolution(20);
    }
    gizmos.line_2d(p - d, p + d * 2.7, color);
    gizmos.line_2d(p + d * 2.7, p + d * 1.5 + d.perp() * 0.7, color);
    gizmos.line_2d(p + d * 2.7, p + d * 1.5 - d.perp() * 0.7, color);
}
fn warehouse(gizmos: &mut Gizmos, offset: f32) {
    let p = |x, y| point(Pose { x, y, heading: 0.0 }, offset);
    for x in -2..=14 {
        gizmos.line_2d(p(x as f64, -2.0), p(x as f64, 10.0), Color::srgba(0.45, 0.6, 0.7, 0.09));
    }
    for y in -2..=10 {
        gizmos.line_2d(p(-2.0, y as f64), p(14.0, y as f64), Color::srgba(0.45, 0.6, 0.7, 0.09));
    }
    gizmos.rect_2d(Isometry2d::from_translation(p(6.0, 4.0)), Vec2::new(16.0 * 5.3, 12.0 * 5.3), MUTED.with_alpha(0.35));
    for x in [3.0, 9.0] {
        gizmos.rect_2d(Isometry2d::from_translation(p(x, 4.0)), Vec2::splat(4.0 * 5.3), MUTED.with_alpha(0.65));
        for y in [3.0, 4.0, 5.0] {
            gizmos.line_2d(p(x - 2.0, y), p(x + 2.0, y), MUTED.with_alpha(0.3));
        }
    }
    gizmos.circle_2d(p(6.0, 6.0), 1.8, Color::srgb(0.62, 0.88, 0.61)).resolution(24);
    gizmos.line_2d(p(5.5, 0.0), p(6.5, 0.0), Color::srgb(0.62, 0.88, 0.61));
}
fn trail(gizmos: &mut Gizmos, estimate: &Estimate, offset: f32, color: Color) {
    for points in estimate.trail.windows(2) {
        gizmos.line_2d(point(points[0].pose, offset), point(points[1].pose, offset), color);
    }
}
fn route(gizmos: &mut Gizmos, points: &[Pose], offset: f32, color: Color) {
    for segment in points.windows(2) {
        let a = point(segment[0], offset);
        let b = point(segment[1], offset);
        let delta = b - a;
        let steps = (delta.length() / 1.8).ceil() as usize;
        for step in (0..steps.min(256)).step_by(2) {
            gizmos.line_2d(a + delta * (step as f32 / steps as f32), a + delta * ((step + 1) as f32 / steps as f32).min(1.0), color);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn same_time_revisions_wait_for_handoff_and_pause_preserves_pending_state() {
        let mut display = Display::default();
        let initial = Report { revision: 1, query_ms: 1000, ..Default::default() };
        display.observe(&initial, true);
        let changed =
            Report { revision: 2, historical: Estimate { pose: Pose { x: 5.0, ..Default::default() }, ..Default::default() }, ..initial };
        display.observe(&changed, false);
        assert_eq!(display.visible.historical.pose.x, 0.0);
        display.observe(&changed, false);
        assert_eq!(display.pending.len(), 1);
        display.clock = 449.0;
        display.observe(&changed, false);
        assert_eq!(display.visible.historical.pose.x, 0.0);
        display.clock = 450.0;
        display.observe(&changed, false);
        assert_eq!(display.visible.historical.pose.x, 5.0);
    }
}
