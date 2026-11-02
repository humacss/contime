//! Case-specific deterministic simulator callbacks; hosts share this exact source.
use crate::{
    definition::Time,
    uptime_example::model::{Status, StatusChanged},
};
use contime::contime_simulator::{EventStream, StreamConfig};
pub fn event(service: usize, sequence: u64, time: Time, status: Status) -> StatusChanged {
    EventStream::new(
        StreamConfig { event_count: 1, seed: sequence ^ 42, settings: () },
        move |_, _| StatusChanged { id: sequence.into(), service_id: 0, time, status },
        move |_, _, event: &mut StatusChanged| {
            event.service_id = service as u128;
        },
    )
    .next()
    .expect("one configured health change")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callbacks_preserve_source_time_identity_and_selected_service() {
        let event = event(3, 77, Time(1234), Status::Degraded);
        assert_eq!((event.service_id, event.id, event.time, event.status), (3, 77, Time(1234), Status::Degraded));
    }
}
