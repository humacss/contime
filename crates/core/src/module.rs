//! Static application definitions shared by managed and manual execution.
use crate::contime_core::{checkpoints, Input};

/// Metadata supplied by an application event, independent of its registration.
pub trait EventId: checkpoints::Event {
    /// Stable identity used for ordering and duplicate suppression.
    fn event_id(&self) -> u128;
}
/// What to do with a duplicate occurrence.
pub enum DuplicateDecision<E: checkpoints::Event> {
    /// Keep the event already in the history.
    KeepExisting,
    /// Replace the existing event with the supplied occurrence.
    ReplaceExisting {
        /// Timestamp of the event currently retained for this ID.
        existing_time: E::Time,
        /// Event to retain in its place.
        event: E,
    },
}
/// Event-specific duplicate selection and snapshot-owned working context.
///
/// When admission tracking is enabled, the first admission is passed to `on_admit`.
/// Every retained duplicate, including one at the same time, is passed to `select_canonical`.
/// Replacements must identify the current event time, have the same ID, represent a real occurrence and remain
/// within the retained horizon. The policy owns any additional state in its
/// context; contexts are independent per event type and snapshot.
///
/// Contexts do not require `Default`, `Clone`, `Send`, or `Sync`. Policies that
/// retain per-ID state should remove it in `prune_context` when an event is pruned.
/// Context mutations are not rolled back if a replacement is rejected.
pub trait EventDuplicatePolicy: EventId {
    /// Working state for this event type in one snapshot history.
    type Context: 'static;
    /// Whether new admissions must initialize policy context.
    /// The derived default sets this to false; custom policies default to true.
    /// Setting it to false skips `on_admit`; a policy may still replace events if it knows the existing time.
    const TRACK_ADMISSIONS: bool = true;
    /// Whether this implementation supplies custom policy behavior.
    #[doc(hidden)]
    const CUSTOM_POLICY: bool = true;
    /// Creates the context for this event type in a snapshot.
    fn create_context() -> Self::Context;
    /// Records an initially admitted event when the policy needs it.
    fn on_admit(_event: &Self, _context: &mut Self::Context)
    where
        Self: Sized,
    {
    }
    /// Removes policy-owned state for an event being pruned.
    fn prune_context(_event: &Self, _context: &mut Self::Context)
    where
        Self: Sized,
    {
    }
    /// Selects whether to keep or replace the canonical event.
    fn select_canonical(_incoming: &Self, _context: &mut Self::Context) -> DuplicateDecision<Self>
    where
        Self: Sized,
    {
        DuplicateDecision::KeepExisting
    }
}
/// An event-policy bridge used by generated aggregate lanes.
#[doc(hidden)]
pub struct EventPolicyBridge<'a, E>(pub &'a E);
/// Dispatches event policy operations, with a default for handwritten IDs.
#[doc(hidden)]
pub trait EventPolicyDispatch<E: EventId> {
    /// Whether the policy needs admission tracking.
    fn policy_track(self) -> bool;
    /// Construct snapshot-local policy context.
    fn policy_context(self) -> Box<dyn std::any::Any>;
    /// Record an admitted event.
    fn policy_admit(self, context: &mut dyn std::any::Any);
    /// Notify context that an event is being pruned.
    fn policy_prune(self, context: &mut dyn std::any::Any);
    /// Select a canonical occurrence.
    fn policy_select(self, context: &mut dyn std::any::Any) -> Option<DuplicateDecision<E>>;
}
impl<E: EventDuplicatePolicy> EventPolicyDispatch<E> for &EventPolicyBridge<'_, E> {
    #[inline(always)]
    fn policy_track(self) -> bool {
        E::TRACK_ADMISSIONS
    }
    fn policy_context(self) -> Box<dyn std::any::Any> {
        Box::new(E::create_context())
    }
    fn policy_admit(self, context: &mut dyn std::any::Any) {
        if let Some(context) = context.downcast_mut::<E::Context>() {
            E::on_admit(self.0, context);
        }
    }
    fn policy_prune(self, context: &mut dyn std::any::Any) {
        if let Some(context) = context.downcast_mut::<E::Context>() {
            E::prune_context(self.0, context);
        }
    }
    fn policy_select(self, context: &mut dyn std::any::Any) -> Option<DuplicateDecision<E>> {
        Some(E::select_canonical(self.0, context.downcast_mut::<E::Context>()?))
    }
}
impl<E: EventId> EventPolicyDispatch<E> for &&EventPolicyBridge<'_, E> {
    #[inline(always)]
    fn policy_track(self) -> bool {
        false
    }
    fn policy_context(self) -> Box<dyn std::any::Any> {
        Box::new(())
    }
    fn policy_admit(self, _context: &mut dyn std::any::Any) {}
    fn policy_prune(self, _context: &mut dyn std::any::Any) {}
    fn policy_select(self, _context: &mut dyn std::any::Any) -> Option<DuplicateDecision<E>> {
        Some(DuplicateDecision::KeepExisting)
    }
}
/// Type-level policy probe for generated aggregate lanes.
#[doc(hidden)]
pub struct EventPolicyType<E>(pub std::marker::PhantomData<fn() -> E>);
/// Probes whether an event type supplies a custom policy.
#[doc(hidden)]
pub trait EventPolicyTypeDispatch {
    /// Reports whether this event has custom policy behavior.
    fn is_custom(self) -> bool;
}
impl<E: EventDuplicatePolicy> EventPolicyTypeDispatch for &EventPolicyType<E> {
    #[inline(always)]
    fn is_custom(self) -> bool {
        E::CUSTOM_POLICY
    }
}
impl<E: EventId> EventPolicyTypeDispatch for &&EventPolicyType<E> {
    #[inline(always)]
    fn is_custom(self) -> bool {
        false
    }
}
/// An event with identity and event-specific duplicate behavior.
pub trait Event: EventId + EventDuplicatePolicy {}
impl<T: EventId + EventDuplicatePolicy> Event for T {}

/// Stable identity of an initialized snapshot, matching its history's routing ID.
pub trait SnapshotId {
    /// Reads this snapshot's identity without choosing an implicit ID policy.
    fn snapshot_id(&self) -> u128;
}

mod context;
mod creation;
pub use creation::SnapshotCreate;
mod event_set;
pub use context::{apply_batch, ApplyContext, SnapshotApply};
pub use event_set::{EventRef, EventSet, EventSetBuilder, EventSource, SnapshotEvents};

/// A complete statically dispatched application definition.
///
/// All nested modules, inputs and snapshots share exactly `Time`. A module is
/// a definition, not a running process; composing it starts no threads.
pub trait Module: 'static {
    /// Validates event registrations before execution starts.
    fn validate() {}
    /// Ordered application time domain, including its explicit minimum.
    type Time: checkpoints::Timestamp + crate::contime_worker::AdvanceTime + 'static;
    /// Registered inputs and their event-to-snapshot routing.
    type EventLanes: Input<Time = Self::Time>;
    /// Registered snapshot states and deterministic application.
    type SnapshotLanes: checkpoints::Snapshot<Time = Self::Time> + checkpoints::ApplyEvents<Self::EventLanes> + 'static;
}
