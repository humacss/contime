/// One input that ConTime could not admit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventRejection {
    pub event_id: u128,
    pub reason: EventRejectionReason,
}

impl EventRejection {
    pub const fn new(event_id: u128, reason: EventRejectionReason) -> Self {
        Self { event_id, reason }
    }
}

/// Reason one input was rejected while the rest of its batch continued.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EventRejectionReason {
    /// The input predates the earliest retained history time.
    BeforeHistoryHorizon,
    /// Retaining the input would exceed the configured memory budget.
    MemoryFull,
}

pub(crate) fn merge_event_rejections(target: &mut Vec<EventRejection>, incoming: Vec<EventRejection>) {
    if incoming.is_empty() {
        return;
    }
    target.extend(incoming);
    target.sort_unstable();
    target.dedup();
}
