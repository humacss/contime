//! Adapt the independent service lane to the shared execution backends.
use super::model::{Service, Status, StatusChanged};
use crate::definition::{Example, Time};
pub struct ServiceExample;
impl Example for ServiceExample {
    type Event = StatusChanged;
    type State = Service;
    const TITLE: &'static str = "Service uptime";
    const VALUE_LABEL: &'static str = "Downtime";
    fn event(id: u128, time: Time, targets: Vec<u128>, value: i64) -> StatusChanged {
        StatusChanged {
            id,
            time,
            service_id: targets.first().copied().unwrap_or(1),
            status: match value.rem_euclid(3) {
                1 => Status::Degraded,
                2 => Status::Down,
                _ => Status::Healthy,
            },
        }
    }
    fn id(state: &Service) -> u128 {
        state.service_id
    }
    fn count(state: &Service) -> u64 {
        state.events
    }
    fn value(state: &Service) -> i64 {
        (state.downtime / 1_000_000) as i64
    }
    fn record(state: &mut Service, event: &StatusChanged) {
        state.apply_change(event);
    }
}
