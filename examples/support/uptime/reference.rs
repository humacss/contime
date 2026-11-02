//! Independent chronological oracle. It never calls the application's apply method.
use crate::{
    definition::Time,
    uptime_example::model::{Interval, Service, Status, StatusChanged},
};
use std::collections::BTreeMap;
pub(super) fn fold(state: &mut Service, event: &StatusChanged) {
    let elapsed = if state.events == 0 { 0 } else { event.time.0.saturating_sub(state.changed_at.0) };
    match state.status {
        Status::Down => state.downtime += elapsed,
        Status::Healthy => state.uptime += elapsed,
        Status::Degraded => {
            state.uptime += elapsed;
            state.degraded += elapsed;
        }
    }
    if elapsed != 0 {
        state.intervals.push(Interval { start: state.changed_at, end: event.time, status: state.status });
        if state.intervals.len() > 256 {
            state.intervals.remove(0);
        }
    }
    state.status = event.status;
    state.changed_at = event.time;
    state.events += 1;
}
struct History {
    anchor: Service,
    events: BTreeMap<(Time, u128), StatusChanged>,
}
pub(super) struct Reference {
    histories: Vec<History>,
}
impl Reference {
    pub fn new(count: usize) -> Self {
        Self {
            histories: (1..=count)
                .map(|id| History { anchor: Service { service_id: id as u128, ..Default::default() }, events: BTreeMap::new() })
                .collect(),
        }
    }
    pub fn insert(&mut self, event: StatusChanged) {
        let history = &mut self.histories[event.service_id as usize - 1];
        if !history.events.values().any(|e| e.id == event.id) {
            history.events.insert((event.time, event.id), event);
        }
    }
    pub fn states(&self, time: Time) -> Vec<Service> {
        self.histories
            .iter()
            .map(|history| {
                let mut state = history.anchor.clone();
                for event in history.events.values().take_while(|e| e.time <= time) {
                    fold(&mut state, event);
                }
                state
            })
            .collect()
    }
    pub fn compact(&mut self, before: Time) {
        for history in &mut self.histories {
            let keys: Vec<_> = history.events.range(..(before, 0)).map(|(key, _)| *key).collect();
            for key in keys {
                fold(&mut history.anchor, &history.events.remove(&key).expect("selected reference input"));
            }
        }
    }
}
