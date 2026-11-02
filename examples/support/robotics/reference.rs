//! Independent chronological estimator fold, including explicit same-time phases.
use crate::{
    definition::Time,
    robotics_example::model::{Maneuver, Pose, Reading, Robot, Sample, SensorEvent},
};
use std::collections::BTreeMap;
pub struct Reference {
    anchor: Robot,
    events: BTreeMap<(Time, u8, u128), SensorEvent>,
}
fn phase(event: &SensorEvent) -> u8 {
    match event.reading {
        Reading::Motion { .. } => 0,
        Reading::Camera(_) => 1,
        Reading::Plan => 2,
    }
}
fn angle(a: f64) -> f64 {
    (a + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}
impl Reference {
    pub fn new() -> Self {
        Self { anchor: Robot { robot_id: 1, ..Default::default() }, events: BTreeMap::new() }
    }
    pub fn insert(&mut self, event: SensorEvent) {
        self.events.insert((event.time, phase(&event), event.id), event);
    }
    pub fn at(&self, time: Time) -> Robot {
        let mut state = self.anchor.clone();
        for event in self.events.values().take_while(|e| e.time <= time) {
            fold(&mut state, event);
        }
        state
    }
    pub fn compact(&mut self, before: Time) {
        let keys: Vec<_> = self.events.range(..(before, 0, 0)).map(|(key, _)| *key).collect();
        for key in keys {
            fold(&mut self.anchor, &self.events.remove(&key).unwrap());
        }
    }
}
fn fold(state: &mut Robot, event: &SensorEvent) {
    match event.reading {
        Reading::Motion { distance, turn } => {
            let orientation = state.pose.heading;
            state.pose.x += orientation.cos() * distance;
            state.pose.y += orientation.sin() * distance;
            state.pose.heading = angle(orientation + turn);
        }
        Reading::Camera(observation) => {
            state.pose.x = state.pose.x + 0.85 * (observation.x - state.pose.x);
            state.pose.y = state.pose.y + 0.85 * (observation.y - state.pose.y);
            state.pose.heading = angle(state.pose.heading + angle(observation.heading - state.pose.heading) * 0.8);
            state.observations += 1;
        }
        Reading::Plan => {
            let rail = if state.pose.y < 4.0 { 0.0 } else { 8.0 };
            state.route = vec![
                state.pose,
                Pose { x: state.pose.x, y: rail, heading: 0.0 },
                Pose { x: 6.0, y: rail, heading: 0.0 },
                Pose { x: 6.0, y: 6.0, heading: 0.0 },
            ];
            state.proposal = if (state.pose.x - 6.0).abs() >= 0.35 {
                if state.pose.x > 6.0 {
                    Maneuver::Backtrack
                } else {
                    Maneuver::Continue
                }
            } else if (state.pose.y - 6.0).abs() < 0.35 {
                Maneuver::AtGoal
            } else {
                Maneuver::Turn
            };
        }
    }
    state.events += 1;
    if state.trail.last().is_some_and(|p| p.time == event.time) {
        state.trail.pop();
    }
    state.trail.push(Sample { time: event.time, pose: state.pose });
    if state.trail.len() > 120 {
        state.trail.remove(0);
    }
}
