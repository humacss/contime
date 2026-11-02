//! Queried paths and delivered-input annotations. Geometry never feeds simulation.
use crate::report::{Arrival, IntendedPath, Point, Report};
use bevy::{math::Isometry2d, prelude::*};

pub fn ordered_trails(report: &Report) -> impl Iterator<Item = &Point> {
    report.trails.iter().filter(|p| !p.current).chain(report.trails.iter().filter(|p| p.current))
}
fn position(p: [f64; 2]) -> Vec2 {
    Vec2::from_array(p.map(|v| v as f32))
}

/// Faint planned paths sit behind queried trails and never move with corrections.
pub fn draw_intended(report: &Report, gizmos: &mut Gizmos) {
    for path in &report.intended_paths {
        for (start, end) in intended_segments(path) {
            for (from, to) in path_dashes(position(start), position(end)) {
                gizmos.line(from.extend(-0.1), to.extend(-0.1), Color::srgba(0.65, 0.65, 0.65, 0.12));
            }
        }
    }
}

// Out-and-back legs share one guide; drawing them twice would brighten the cross.
fn intended_segments(path: &IntendedPath) -> impl Iterator<Item = ([f64; 2], [f64; 2])> + '_ {
    path.points
        .windows(2)
        .enumerate()
        .filter(|(index, segment)| {
            segment[0] != segment[1]
                && !path
                    .points
                    .windows(2)
                    .take(*index)
                    .any(|earlier| earlier == *segment || (earlier[0] == segment[1] && earlier[1] == segment[0]))
        })
        .map(|(_, segment)| (segment[0], segment[1]))
}

pub fn draw(report: &Report, reduced_motion: bool, gizmos: &mut Gizmos) {
    // Solid, opaque current paths cover replaced geometry at exact overlaps.
    for segment in ordered_trails(report) {
        let Some(start) = segment.previous else {
            continue;
        };
        let strength = segment.opacity as f32;
        let start = position(start);
        let end = position(segment.position);
        if segment.current {
            gizmos.line_2d(start, end, crate::palette::path(segment.client, strength));
        } else {
            for (from, to) in path_dashes(start, end) {
                gizmos.line_2d(from, to, crate::palette::revision(segment.client, strength));
            }
        }
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
// Both patterns stay anchored to world coordinates across queries and reversals.
fn segment_coordinates(start: Vec2, end: Vec2) -> (Vec2, Vec2, f32, f32) {
    let mut axis = (end - start).normalize_or_zero();
    if axis.x < 0.0 || (axis.x == 0.0 && axis.y < 0.0) {
        axis = -axis;
    }
    let anchor = start.dot(axis);
    let other = end.dot(axis);
    (start - axis * anchor, axis, anchor.min(other), anchor.max(other))
}

fn path_dashes(start: Vec2, end: Vec2) -> impl Iterator<Item = (Vec2, Vec2)> {
    const PERIOD: f32 = 1.5;
    const LENGTH: f32 = 0.85;
    let (origin, axis, lo, hi) = segment_coordinates(start, end);
    let first = (lo / PERIOD).floor() as i32;
    let last = (hi / PERIOD).ceil() as i32;
    (first..last).filter_map(move |index| {
        let from = (index as f32 * PERIOD).max(lo);
        let to = (index as f32 * PERIOD + LENGTH).min(hi);
        (to > from).then(|| (origin + axis * from, origin + axis * to))
    })
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
    fn return_trips_and_stops_do_not_brighten_the_same_intended_segment() {
        let path = IntendedPath { client: 1, points: vec![[0.0, 0.0], [0.0, 8.0], [0.0, 0.0], [8.0, 0.0], [0.0, 0.0], [0.0, 0.0]] };
        assert_eq!(intended_segments(&path).collect::<Vec<_>>(), vec![([0.0, 0.0], [0.0, 8.0]), ([0.0, 0.0], [8.0, 0.0]),]);
    }
    #[test]
    fn guide_dashes_are_stable_across_sampling_and_direction_changes() {
        let start = Vec2::new(-3.0, 0.0);
        let end = Vec2::new(3.0, 0.0);
        let whole: Vec<_> = path_dashes(start, end).collect();
        assert_eq!(whole, path_dashes(end, start).collect::<Vec<_>>());
        let split = Vec2::new(0.85, 0.0);
        let pieces: Vec<_> = path_dashes(start, split).chain(path_dashes(split, end)).collect();
        assert_eq!(whole, pieces);
        assert_eq!(path_dashes(start, start).count(), 0);
        assert!(path_dashes(Vec2::ZERO, Vec2::Y * 3.0).all(|(a, b)| a.x == 0.0 && b.x == 0.0 && a.y < b.y));
        assert!(whole.iter().all(|(a, b)| b.x > a.x && b.x - a.x <= 0.850001));
    }
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
