/// One local measurement, not an irreversible or distributed safe frontier.
/// New admission may create work before `before` after this report is produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgressObservation<T> {
    pub round: u64,
    pub cutoff: T,
    /// Exclusive boundary, capped at `cutoff`.
    pub before: T,
}

/// Consumer policy installed when a processing pipeline starts.
///
/// The host owns ordered measurement and resumption. A policy selects a bound
/// and consumes reports; it does not grant permission to prune or reject input.
/// Callbacks run on the host coordinator and must be short, nonblocking and
/// must not synchronously wait for work on that same pipeline.
pub trait ProgressPolicy<T>: Send + 'static {
    /// Called on startup, advancement, admission and incomplete measurements.
    /// Returning `None` disables observation until another such change.
    fn cutoff(&mut self, advanced: &T) -> Option<T>;

    /// Discard cached observations before newly admitted work is routed.
    /// Called once per nonempty admitted batch, including internal feedback.
    /// A later observation accounts for that admission, conservatively when it
    /// arrives during an active round. Queues outside the host remain the
    /// caller's responsibility. Empty and wholly rejected batches do not notify.
    fn invalidated(&mut self) {}

    fn observed(&mut self, report: ProgressObservation<T>);
}

impl<T> ProgressPolicy<T> for () {
    fn cutoff(&mut self, _: &T) -> Option<T> {
        None
    }
    fn observed(&mut self, _: ProgressObservation<T>) {}
}
