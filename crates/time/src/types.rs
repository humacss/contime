/// A discrete ordered time domain with an explicit lower bound.
///
/// `Default` is an application convenience and need not equal the minimum.
/// All values at or above `minimum`, including negative values and zero, are valid.
pub trait Timestamp: Clone + Default + Ord {
    /// The least valid timestamp. No representable valid time is earlier.
    fn minimum() -> Self;
    /// The immediately preceding timestamp, or `None` exactly at the minimum.
    fn previous(&self) -> Option<Self>;
}
/// Retention arithmetic over an ordered time domain.
pub trait AdvanceTime: Timestamp {
    /// Subtracts a nonnegative retention interval, saturating at the domain minimum.
    fn saturating_sub(&self, retention: &Self) -> Self;
    /// Whether this value is a valid, nonnegative retention interval.
    fn is_valid_retention(&self) -> bool;
}
