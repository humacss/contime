//! Decimal average target count, represented exactly to three decimal places.
use std::{fmt, str::FromStr};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Average(u64);
impl Average {
    pub const ONE: Self = Self(1000);
    pub fn from_scaled(value: u64) -> Self {
        assert!((1000..=1_000_000_000).contains(&value));
        Self(value)
    }
    pub fn scaled(self) -> u64 {
        self.0
    }
}
impl Default for Average {
    fn default() -> Self {
        Self::ONE
    }
}
impl FromStr for Average {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let invalid = || "Average snapshots/event must be 1–1000000, with up to three decimal places".to_string();
        let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
        if whole.is_empty()
            || !whole.bytes().all(|c| c.is_ascii_digit())
            || fraction.len() > 3
            || !fraction.bytes().all(|c| c.is_ascii_digit())
        {
            return Err(invalid());
        }
        let whole = whole.parse::<u64>().map_err(|_| invalid())?;
        let fraction =
            if fraction.is_empty() { 0 } else { fraction.parse::<u64>().map_err(|_| invalid())? * 10u64.pow(3 - fraction.len() as u32) };
        let scaled = whole.checked_mul(1000).and_then(|n| n.checked_add(fraction)).ok_or_else(invalid)?;
        if !(1000..=1_000_000_000).contains(&scaled) {
            return Err(invalid());
        }
        Ok(Self(scaled))
    }
}
impl fmt::Display for Average {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let fraction = format!("{:03}", self.0 % 1000);
        let fraction = fraction.trim_end_matches('0');
        write!(f, "{}.{}", self.0 / 1000, if fraction.is_empty() { "0" } else { fraction })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn average_parsing_preserves_decimal_precision_and_rejects_invalid_input() {
        for (text, scaled) in [("1", 1000), ("1.1", 1100), ("2.375", 2375), ("1000000", 1000000000)] {
            let value: Average = text.parse().unwrap();
            assert_eq!(value.scaled(), scaled);
            assert_eq!(value.to_string().parse::<Average>().unwrap(), value);
        }
        for text in ["0", "0.999", "NaN", "inf", "-1", "1e3", "1.0001", "1..1", "", "18446744073709551616"] {
            assert!(text.parse::<Average>().is_err(), "{text}");
        }
    }
}
