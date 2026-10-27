//! Human-readable numeric formatting shared by native example screens.
use std::fmt::Display;
/// Group whole-number digits, preserving any sign and fractional part.
pub fn number(value: impl Display) -> String {
    let text = value.to_string();
    let sign_len = usize::from(text.starts_with('-') || text.starts_with('+'));
    let (sign, rest) = text.split_at(sign_len);
    let (whole, fraction) = rest.find('.').map_or((rest, ""), |i| rest.split_at(i));
    if whole.is_empty() || !whole.bytes().all(|c| c.is_ascii_digit()) {
        return text;
    }
    let mut out = String::with_capacity(text.len() + whole.len() / 3);
    out.push_str(sign);
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out.push_str(fraction);
    out
}
/// Round first, then group digits without losing the requested precision.
pub fn decimal(value: f64, precision: usize) -> String {
    number(format!("{value:.precision$}"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn groups_counts_and_preserves_sign_without_overflow() {
        assert_eq!(number(0), "0");
        assert_eq!(number(999), "999");
        assert_eq!(number(1000), "1,000");
        assert_eq!(number(1_234_567), "1,234,567");
        assert_eq!(number(i64::MIN), "-9,223,372,036,854,775,808");
        assert_eq!(number(u64::MAX), "18,446,744,073,709,551,615");
    }
    #[test]
    fn groups_rates_and_timings_after_rounding() {
        assert_eq!(decimal(1234567.25, 1), "1,234,567.2");
        assert_eq!(decimal(999.999, 2), "1,000.00");
        assert_eq!(decimal(-1234.5, 1), "-1,234.5");
        assert_eq!(decimal(0.012, 3), "0.012");
    }
}
