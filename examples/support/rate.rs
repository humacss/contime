//! Synthetic production policy, independent of ConTime processing.
use std::fmt;
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EventRate {
    #[default]
    Maximum,
    PerSecond(usize),
}
impl EventRate {
    pub fn limit(self) -> Option<usize> {
        match self {
            Self::Maximum => None,
            Self::PerSecond(rate) => Some(rate),
        }
    }
    #[allow(dead_code)] // Portable hosts have no terminal settings form.
    pub(super) fn form_value(self) -> u64 {
        self.limit().map_or(0, |n| n as u64)
    }
}
impl fmt::Display for EventRate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Maximum => f.write_str("Maximum"),
            Self::PerSecond(rate) => write!(f, "{}/s", super::number(rate)),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maximum_and_numeric_rates_preserve_their_limits_and_formatting() {
        assert_eq!(EventRate::PerSecond(1_234_567).to_string(), "1,234,567/s");
        assert_eq!(EventRate::Maximum.to_string(), "Maximum");
        assert_eq!(EventRate::Maximum.limit(), None);
        assert_eq!(EventRate::PerSecond(100).limit(), Some(100));
    }
}
