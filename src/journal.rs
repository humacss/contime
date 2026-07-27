use crate::Event;

/// One canonical original event retained for inspection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventJournalEntry<E> {
    /// The original event lane submitted to ConTime.
    pub event: E,
    /// Snapshot ids selected by the event lane's routing.
    pub routed_snapshot_ids: Vec<u128>,
}

impl<E> EventJournalEntry<E>
where
    E: Event,
{
    pub(crate) fn conservative_size(&self) -> u64 {
        self.event.conservative_size().saturating_add((self.routed_snapshot_ids.len() * size_of::<u128>()) as u64)
    }
}
