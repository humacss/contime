//! Equal-scale views of the same input deliveries; geometry never affects state.
use crate::report::Report;
pub const OFFSET: f64 = 52.0;

pub fn panels(report: &Report) -> (Report, Report) {
    let mut left = report.clone();
    let mut right = report.clone();
    right.positions = report.unreconciled_positions.clone();
    right.trails = report.unreconciled_trails.clone();
    right.arrivals = report.unreconciled_arrivals.clone();
    for (view, offset) in [(&mut left, -OFFSET), (&mut right, OFFSET)] {
        for path in &mut view.intended_paths {
            for point in &mut path.points {
                point[0] += offset;
            }
        }
        for client in &mut view.positions {
            client.position[0] += offset;
        }
        for point in &mut view.trails {
            point.position[0] += offset;
            if let Some(previous) = &mut point.previous {
                previous[0] += offset;
            }
        }
        for arrival in &mut view.arrivals {
            arrival.position[0] += offset;
        }
        view.unreconciled_positions.clear();
        view.unreconciled_trails.clear();
        view.unreconciled_arrivals.clear();
    }
    (left, right)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn both_views_share_scale_and_identity_without_replacing_queried_state() {
        let mut source = crate::animation::tests::scene(3.0);
        source.unreconciled_positions = source.positions.clone();
        source.unreconciled_positions[0].position = [7.0, 2.0];
        source.intended_paths.push(crate::report::IntendedPath { client: 1, points: vec![[0.0, 0.0], [8.0, 0.0]] });
        let (left, right) = panels(&source);
        assert_eq!(left.positions[0].position, [3.0 - OFFSET, 0.0]);
        assert_eq!(right.positions[0].position, [7.0 + OFFSET, 2.0]);
        assert_eq!(left.arena_half_extent, right.arena_half_extent);
        assert_eq!(left.positions[0].id, right.positions[0].id);
        assert_eq!(source.positions[0].position, [3.0, 0.0]);
        assert_eq!(left.intended_paths[0].points, vec![[-OFFSET, 0.0], [8.0 - OFFSET, 0.0]]);
        assert_eq!(right.intended_paths[0].points, vec![[OFFSET, 0.0], [8.0 + OFFSET, 0.0]]);
        assert_eq!(source.intended_paths[0].points, vec![[0.0, 0.0], [8.0, 0.0]]);
    }
    #[test]
    fn invalid_comparison_data_is_rejected_before_rendering() {
        let mut source = crate::animation::tests::scene(0.0);
        source.unreconciled_positions = source.positions.clone();
        source.unreconciled_positions[0].position[0] = f64::NAN;
        assert!(source.validate().is_err());
        source.unreconciled_positions[0].position[0] = 1.0;
        source.unreconciled_positions.push(source.unreconciled_positions[0].clone());
        assert!(source.validate().is_err());
    }
    #[test]
    fn intended_guides_are_bounded_finite_and_unique_per_client() {
        let mut source = crate::animation::tests::scene(0.0);
        let path = crate::report::IntendedPath { client: 1, points: vec![[0.0, 0.0], [8.0, 0.0]] };
        source.intended_paths.push(path.clone());
        assert!(source.validate().is_ok());
        source.intended_paths[0].points[1][0] = f64::NAN;
        assert!(source.validate().is_err());
        source.intended_paths[0] = path.clone();
        source.intended_paths[0].points[1][0] = 49.0;
        assert!(source.validate().is_err());
        source.intended_paths[0] = path.clone();
        source.intended_paths[0].points = vec![[0.0, 0.0]; 10];
        assert!(source.validate().is_err());
        source.intended_paths[0] = path.clone();
        source.intended_paths.push(path);
        assert!(source.validate().is_err());
    }
}
