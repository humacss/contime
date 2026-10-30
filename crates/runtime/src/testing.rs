#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub struct Time(pub i64);

impl crate::contime_runtime::checkpoints::Timestamp for Time {
    // This fixture models a nonnegative logical clock.
    fn minimum() -> Self {
        Self(0)
    }
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).filter(|time| *time >= 0).map(Self)
    }
}
impl crate::contime_worker::AdvanceTime for Time {
    fn is_valid_retention(&self) -> bool {
        self.0 >= 0
    }
    fn saturating_sub(&self, other: &Self) -> Self {
        Self(self.0.saturating_sub(other.0).max(0))
    }
}
