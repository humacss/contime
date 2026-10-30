//! Monitor-confirmed historical query cycles and full-character validation.
use super::reference::Reference;
use super::render_state::{Rendering, Sample};
use crate::{
    definition::Time,
    movement_example::{model::Character, setup::MovementExample},
    support::monitored::backend::Backend,
};
use contime::contime_monitor::WorkerReading;
use crossbeam_channel::{Receiver, TryRecvError};
use std::{collections::VecDeque, time::Duration};
type Failure = Box<dyn std::error::Error>;
pub(super) struct Seal {
    pub time: Time,
    pub submitted: u64,
    pub final_query: bool,
    pub started: Duration,
}
struct Query {
    #[allow(clippy::vec_box)] // Existing ConTime response transport owns boxed snapshots.
    receiver: Receiver<Vec<Box<Character>>>,
    time: Time,
    actual: Vec<Character>,
    seen: Vec<bool>,
}
pub(super) struct Cycle {
    remaining: VecDeque<Time>,
    query: Option<Query>,
    pub rendering: Rendering,
    pub seal: Seal,
}
impl Cycle {
    pub fn new(seal: Seal, trail: u64) -> Self {
        Self { remaining: times(seal.time, trail), query: None, rendering: Rendering::default(), seal }
    }
}
pub(super) fn ready(reading: &WorkerReading<Time>, seal: &Seal) -> bool {
    let inserted = reading
        .workers
        .iter()
        .try_fold(0u64, |sum, w| w.counters.filter(|c| !c.saturated).and_then(|c| sum.checked_add(c.inserted_applications)));
    inserted == Some(seal.submitted) && reading.workers.iter().all(|w| w.pending_histories == 0 && w.processing_target >= seal.time)
}
fn times(now: Time, trail: u64) -> VecDeque<Time> {
    let step = 200_000_000;
    let first = now.0.saturating_sub(trail * 1_000_000).div_ceil(step) * step;
    let mut times: VecDeque<_> = (first..=now.0).step_by(step as usize).map(Time).collect();
    if times.back() != Some(&now) {
        times.push_back(now)
    }
    times
}
pub(super) fn poll_cycle<B: Backend<MovementExample>>(
    cycle: &mut Cycle,
    backend: &B,
    reference: &Reference,
    clients: usize,
    now: Duration,
) -> Result<bool, Failure> {
    if now.saturating_sub(cycle.seal.started) > Duration::from_secs(10) {
        return Err("Rendering query cycle timed out".into());
    }
    if let Some(query) = &mut cycle.query {
        loop {
            match query.receiver.try_recv() {
                Ok(states) => {
                    for state in states {
                        let index = usize::try_from(state.client_id)
                            .ok()
                            .and_then(|id| id.checked_sub(1))
                            .filter(|&id| id < clients)
                            .ok_or("Unknown queried client")?;
                        if query.seen[index] {
                            return Err("Duplicate queried character".into());
                        }
                        query.seen[index] = true;
                        query.actual[index] = *state;
                    }
                }
                Err(TryRecvError::Empty) => return Ok(false),
                Err(TryRecvError::Disconnected) => break,
            }
        }
        let expected = reference.states(query.time);
        for (actual, expected) in query.actual.iter().zip(&expected) {
            if actual.events != expected.events
                || actual.position_at(query.time) != expected.position_at(query.time)
                || actual.direction != expected.direction
                || actual.motion != expected.motion
            {
                return Err(format!("Client {} differs from canonical movement reference", expected.client_id).into());
            }
        }
        cycle
            .rendering
            .samples
            .push(Sample { time: query.time, positions: query.actual.iter().map(|s| s.position_at(query.time)).collect() });
        // Retain headings from the validated query, not screen-space motion.
        cycle.rendering.directions = query.actual.iter().map(|s| s.direction.velocity(1).map(|v| v as f64)).collect();
        cycle.query = None;
    }
    if let Some(time) = cycle.remaining.pop_front() {
        let (sender, receiver) = crossbeam_channel::unbounded();
        backend.query(time, clients, sender)?;
        cycle.query = Some(Query { receiver, time, actual: reference.initial_states(), seen: vec![false; clients] });
        Ok(false)
    } else {
        Ok(true)
    }
}
