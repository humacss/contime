//! Adapt a derived event/snapshot pair to hosts that use concrete types.
//!
//! IDs, destinations, initialization and application remain in the case derives.
//! This adapter adds no scheduling, serialization or alternative ID policy.
#[allow(unused_macros)] // The launcher can compile support without constructing a case.
macro_rules! single_lane {
    ($event:ty => $snapshot:ty) => {
        impl contime::module::View<$event> for $event {
            fn get(&self) -> Option<&$event> {
                Some(self)
            }
        }
        impl contime::Input for $event {
            fn event_id(&self) -> u128 {
                contime::module::Event::event_id(self)
            }
            fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
                <$snapshot as contime::module::Route<$event>>::snapshot_ids(self, emit);
            }
        }
        impl contime::checkpoints::ApplyEvents<$event> for $snapshot {
            fn create(id: u128, event: &$event) -> Self {
                // Hosts call create only for IDs emitted by the same event's route.
                <$snapshot as contime::module::Route<$event>>::create(id, event)
                    .expect("snapshot ID must belong to the event's derived route")
            }
            fn apply_events(&mut self, batch: contime::checkpoints::ApplyBatch<'_, '_, Self::Time, $event>) {
                <Self as contime::module::Apply<$event>>::apply(self, batch);
            }
        }
    };
}
pub(crate) use single_lane;
