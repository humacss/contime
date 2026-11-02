use super::model::*;
use crate::definition::Time;
#[test]
fn heading_correction_changes_subsequent_body_relative_motion() {
    let mut state = Robot::default();
    state.apply_reading(&SensorEvent { id: 1, robot_id: 1, time: Time(1), reading: Reading::Motion { distance: 1.0, turn: 0.2 } });
    state.apply_reading(&SensorEvent {
        id: 2,
        robot_id: 1,
        time: Time(2),
        reading: Reading::Camera(Pose { x: 1.0, y: 0.0, heading: 0.0 }),
    });
    state.apply_reading(&SensorEvent { id: 3, robot_id: 1, time: Time(3), reading: Reading::Motion { distance: 1.0, turn: 0.0 } });
    assert!((state.pose.heading - 0.04).abs() < 1e-12);
    assert!((state.pose.x - (1.0 + 0.04f64.cos())).abs() < 1e-12);
    assert!((state.pose.y - 0.04f64.sin()).abs() < 1e-12);
}
#[test]
fn planning_uses_estimated_pose_and_does_not_move_the_physical_robot() {
    let mut robot = Robot { pose: Pose { x: 7.0, y: 0.0, heading: 0.0 }, ..Default::default() };
    robot.plan();
    assert_eq!(robot.proposal, Maneuver::Backtrack);
    robot.pose.x = 5.5;
    robot.plan();
    assert_eq!(robot.proposal, Maneuver::Continue);
    robot.pose.x = 6.0;
    robot.plan();
    assert_eq!(robot.proposal, Maneuver::Turn);
    assert_eq!(robot.pose.x, 6.0);
    let planned = robot.route.clone();
    robot.apply_reading(&SensorEvent { id: 4, robot_id: 1, time: Time(4), reading: Reading::Motion { distance: 0.1, turn: 0.0 } });
    assert_eq!(robot.route, planned, "a motion event cannot invent a new route between planning ticks");
}
