//! Known physical motion only generates sensor data and the visualization's ground truth.
//! The estimator and planner cannot consult it.
use crate::{
    definition::Time,
    robotics_example::model::{wrap, Pose, Reading, SensorEvent},
};
use contime::contime_simulator::{EventStream, StreamConfig};
pub fn truth(ms: u64) -> Pose {
    let t = (ms % 40000) as f64 / 1000.0;
    if t < 12.0 {
        Pose { x: t, y: 0.0, heading: 0.0 }
    } else if t < 20.0 {
        Pose { x: 12.0, y: t - 12.0, heading: std::f64::consts::FRAC_PI_2 }
    } else if t < 32.0 {
        Pose { x: 32.0 - t, y: 8.0, heading: std::f64::consts::PI }
    } else {
        Pose { x: 0.0, y: 40.0 - t, heading: -std::f64::consts::FRAC_PI_2 }
    }
}
pub fn action_ns(index: u64) -> u64 {
    (index / 3 + 1).saturating_mul(100_000_000).saturating_add(1)
}
pub fn input(index: u64) -> Option<(SensorEvent, u64)> {
    let tick = index / 3 + 1;
    let ms = tick * 100;
    let (reading, delay) = match index % 3 {
        0 => {
            let slip = (1000..3000).contains(&(ms % 40000));
            (
                Reading::Motion {
                    distance: if slip { 0.2 } else { 0.1 },
                    turn: wrap(truth(ms).heading - truth(ms - 100).heading) + if slip { 0.006 } else { 0.0 },
                },
                0,
            )
        }
        1 if tick >= 40 && (tick.is_multiple_of(40) || tick % 40 == 10) => {
            (Reading::Camera(truth(ms)), if tick.is_multiple_of(40) { 1800 } else { 400 })
        }
        2 if tick.is_multiple_of(5) => (Reading::Plan, 0),
        _ => return None,
    };
    Some((
        EventStream::new(
            StreamConfig { event_count: 1, seed: index, settings: () },
            move |_, _| SensorEvent { id: u128::from(index) + 1, robot_id: 0, time: Time(action_ns(index)), reading: reading.clone() },
            |_, _, event: &mut SensorEvent| event.robot_id = 1,
        )
        .next()
        .expect("one sensor event"),
        delay,
    ))
}
