//! A deliberately simple estimator: body-frame odometry and weighted landmark poses.
//! Ground truth is never available to these application rules.
use crate::definition::Time;
use contime::prelude::*;
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pose {
    pub x: f64,
    pub y: f64,
    pub heading: f64,
}
#[derive(Clone, Debug)]
pub enum Reading {
    Motion { distance: f64, turn: f64 },
    Camera(Pose),
    Plan,
}
#[derive(Clone, Debug, ContimeEvent)]
pub struct SensorEvent {
    #[contime(id)]
    pub id: u128,
    pub robot_id: u128,
    #[contime(time)]
    pub time: Time,
    pub reading: Reading,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Maneuver {
    #[default]
    Continue,
    Backtrack,
    Turn,
    AtGoal,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub time: Time,
    pub pose: Pose,
}
#[derive(Clone, Debug, Default, ContimeSnapshot)]
#[contime(events=[SensorEvent])]
pub struct Robot {
    #[contime(id)]
    pub robot_id: u128,
    #[contime(time)]
    pub time: Time,
    pub pose: Pose,
    pub proposal: Maneuver,
    pub route: Vec<Pose>,
    pub events: u64,
    pub observations: u64,
    pub trail: Vec<Sample>,
}
pub fn wrap(angle: f64) -> f64 {
    (angle + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}
impl Robot {
    pub fn apply_reading(&mut self, event: &SensorEvent) {
        match event.reading {
            // Integrate motion through T using the preceding body orientation;
            // a turn at T establishes orientation for the next interval.
            Reading::Motion { distance, turn } => {
                self.pose.x += distance * self.pose.heading.cos();
                self.pose.y += distance * self.pose.heading.sin();
                self.pose.heading = wrap(self.pose.heading + turn);
            }
            Reading::Camera(measured) => {
                self.pose.x += 0.85 * (measured.x - self.pose.x);
                self.pose.y += 0.85 * (measured.y - self.pose.y);
                self.pose.heading = wrap(self.pose.heading + 0.8 * wrap(measured.heading - self.pose.heading));
                self.observations += 1;
            }
            Reading::Plan => self.plan(),
        }
        self.events += 1;
        let sample = Sample { time: event.time, pose: self.pose };
        if self.trail.last().is_some_and(|last| last.time == event.time) {
            self.trail.pop();
        }
        self.trail.push(sample);
        if self.trail.len() > 120 {
            self.trail.remove(0);
        }
    }
    pub fn plan(&mut self) {
        let rail = if self.pose.y < 4.0 { 0.0 } else { 8.0 };
        self.route = vec![
            self.pose,
            Pose { x: self.pose.x, y: rail, heading: 0.0 },
            Pose { x: 6.0, y: rail, heading: 0.0 },
            Pose { x: 6.0, y: 6.0, heading: 0.0 },
        ];
        self.proposal = if (self.pose.x - 6.0).abs() < 0.35 {
            if (self.pose.y - 6.0).abs() < 0.35 {
                Maneuver::AtGoal
            } else {
                Maneuver::Turn
            }
        } else if self.pose.x > 6.0 {
            Maneuver::Backtrack
        } else {
            Maneuver::Continue
        };
    }
}
impl SnapshotApply for Robot {
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        // Explicit same-time phases: motion through T, observation at T, planning at T.
        for phase in 0..3 {
            for event in batch.events::<SensorEvent>() {
                if phase
                    == match event.reading {
                        Reading::Motion { .. } => 0,
                        Reading::Camera(_) => 1,
                        Reading::Plan => 2,
                    }
                {
                    self.apply_reading(event);
                }
            }
        }
    }
}
crate::definition::single_lane!(SensorEvent => Robot);
