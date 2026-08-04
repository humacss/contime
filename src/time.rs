use std::fmt::Debug;
use std::ops::{Add, Sub};
use std::time::Duration;

/// A totally ordered time value used by events, snapshots, histories, and queries.
///
/// Addition and subtraction are intentionally closed over the same type. Consumers
/// define how arithmetic affects composite ordering components.
pub trait ContimeTime: Clone + Default + Ord + Eq + Add<Output = Self> + Sub<Output = Self> + Send + Sync + Debug + 'static {
    /// Subtracts a history horizon without overflowing the time representation.
    fn saturating_sub(self, rhs: Self) -> Self;
}

macro_rules! impl_integer_time {
    ($($time:ty),+ $(,)?) => {
        $(
            impl ContimeTime for $time {
                fn saturating_sub(self, rhs: Self) -> Self {
                    <$time>::saturating_sub(self, rhs)
                }
            }
        )+
    };
}

impl_integer_time!(i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize);

impl ContimeTime for Duration {
    fn saturating_sub(self, rhs: Self) -> Self {
        Duration::saturating_sub(self, rhs)
    }
}
