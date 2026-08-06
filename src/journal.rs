use crate::Input;

/// One canonical temporal input retained for inspection.
///
/// Journal entries follow the same horizon-based retention as snapshot
/// history and are not a persistent input store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputJournalEntry<I> {
    /// The original input lane submitted to ConTime.
    pub input: I,
    /// Snapshot ids selected by the input lane's routing.
    pub routed_snapshot_ids: Vec<u128>,
}

impl<I> InputJournalEntry<I>
where
    I: Input,
{
    pub(crate) fn conservative_size(&self) -> u64 {
        Input::conservative_size(&self.input).saturating_add((self.routed_snapshot_ids.len() * size_of::<u128>()) as u64)
    }
}
