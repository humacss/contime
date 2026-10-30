//! Queried paths and delivered-input annotations. Geometry never feeds simulation.
use crate::report::{Arrival, Point, Report};
use bevy::{math::Isometry2d, prelude::*};

pub fn ordered_trails(report: &Report) -> impl Iterator<Item = &Point> {
    report.trails.iter().filter(|p| !p.current).chain(report.trails.iter().filter(|p| p.current))
}
fn position(p: [f64; 2]) -> Vec2 {
    Vec2::from_array(p.map(|v| v as f32))
}

pub fn draw(report: &Report, reduced_motion: bool, gizmos: &mut Gizmos) {
    // Opaque client paths are drawn last, so shared geometry hides old gray exactly.
    for segment in ordered_trails(report) {
        let Some(start) = segment.previous else {
            continue;
        };
        let strength = segment.opacity as f32;
        let color =
            if segment.current { crate::palette::path(segment.client, strength) } else { Color::srgba(0.60, 0.60, 0.60, strength * 0.5) };
        gizmos.line_2d(position(start), position(segment.position), color);
    }
    for input in &report.arrivals {
        let p = position(input.position);
        let direction = position(input.direction);
        let age = input.opacity as f32;
        let scale = marker_scale(input, reduced_motion);
        let color = crate::palette::input(input.client, 0.35 + 0.65 * age);
        if direction == Vec2::ZERO {
            gizmos.rect_2d(Isometry2d::from_translation(p), Vec2::splat(0.42 * scale), color);
        } else {
            let (tip, left, right) = arrow(input, scale);
            gizmos.line_2d(p - direction * (0.25 * scale), tip, color);
            gizmos.line_2d(left, tip, color);
            gizmos.line_2d(right, tip, color);
        }
    }
}
// Historical age sets the resting size; arrival adds a short ease-out boost.
fn marker_scale(input: &Arrival, reduced_motion: bool) -> f32 {
    let boost = if reduced_motion { 0.0 } else { 0.9 * (input.pulse as f32).powi(2) };
    0.45 + 0.85 * input.opacity as f32 + boost
}
fn arrow(input: &Arrival, scale: f32) -> (Vec2, Vec2, Vec2) {
    let p = position(input.position);
    let d = position(input.direction);
    let tip = p + d * (0.7 * scale);
    let shoulder = tip - d * (0.3 * scale);
    (tip, shoulder + d.perp() * (0.2 * scale), shoulder - d.perp() * (0.2 * scale))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arrows_use_the_commanded_direction_and_historical_position() {
        let input = Arrival { id: 9, client: 1, time_ns: 10, position: [4.0, 5.0], direction: [-1.0, 0.0], opacity: 1.0, pulse: 1.0 };
        let (tip, a, b) = arrow(&input, 1.0);
        assert!(tip.x < 4.0);
        assert_eq!(tip.y, 5.0);
        assert!((a.y - 5.0).abs() == (b.y - 5.0).abs());
    }
    #[test]
    fn marker_size_shows_input_age_and_arrival_without_rings() {
        let mut input = Arrival { id: 1, client: 1, time_ns: 10, position: [0.0, 0.0], direction: [1.0, 0.0], opacity: 1.0, pulse: 0.0 };
        let recent = marker_scale(&input, false);
        input.opacity = 0.2;
        assert!(marker_scale(&input, false) < recent);
        input.opacity = 1.0;
        input.pulse = 1.0;
        assert!(marker_scale(&input, false) > recent);
        assert_eq!(marker_scale(&input, true), recent);
    }
    #[test]
    fn malformed_and_unbounded_arrivals_are_rejected() {
        let mut report = crate::animation::tests::scene(0.0);
        let input = Arrival { id: 1, client: 1, time_ns: 10, position: [0.0, 0.0], direction: [0.0, 0.0], opacity: 1.0, pulse: 0.5 };
        report.arrivals.push(input.clone());
        assert!(report.validate().is_ok());
        report.arrivals[0].pulse = f64::NAN;
        assert!(report.validate().is_err());
        report.arrivals = vec![input; 129];
        assert!(report.validate().is_err());
    }
}
