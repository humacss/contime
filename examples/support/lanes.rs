//! Adapt a trait-based event/snapshot pair to hosts that use concrete types.
//!
//! IDs, destinations, initialization and application remain in the case's trait impls.
//! This adapter adds no scheduling, serialization or alternative ID policy.
#[allow(unused_macros)] // The launcher can compile support without constructing a case.
macro_rules! single_lane {
    ($event:ty => $snapshot:ty) => {
        impl $snapshot {
            fn event_set() -> &'static contime::module::EventSet<Self> {
                static SET: std::sync::OnceLock<contime::module::EventSet<$snapshot>> = std::sync::OnceLock::new();
                SET.get_or_init(|| {
                    let set = contime::module::EventSet::new();
                    set.validate::<$event>();
                    set
                })
            }
        }
        impl contime::Input for $event {
            fn event_id(&self) -> u128 {
                contime::module::Event::event_id(self)
            }
            fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
                <$snapshot>::event_set().snapshot_ids(contime::EventRef::new(self), emit);
            }
        }
        impl contime::checkpoints::ApplyEvents<$event> for $snapshot {
            fn create(id: u128, event: &$event) -> Self {
                // Hosts call create only for IDs emitted by the same event's route.
                <$snapshot>::event_set()
                    .create(id, contime::EventRef::new(event))
                    .expect("snapshot ID must belong to the event's registered route")
            }
            fn apply_events(&mut self, batch: contime::checkpoints::ApplyBatch<'_, '_, Self::Time, $event>) {
                contime::module::apply_batch(self, batch);
            }
        }
    };
}
pub(crate) use single_lane;
