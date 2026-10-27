//! Setting metadata and value normalization; no editor state lives here.
use crate::support::Average;

#[derive(Clone, Copy)]
pub enum ValueKind {
    Integer { min: u64, max: u64 },
    Decimal { min: f64, max: f64 },
    Average { max: u64 },
    Rate,
}
#[derive(Clone)]
pub struct Definition {
    pub id: &'static str,
    pub group: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub unit: &'static str,
    pub kind: ValueKind,
    pub choices: Vec<String>,
    pub default: String,
}
impl Definition {
    pub fn normalize(&self, value: &str) -> Result<String, String> {
        let invalid = || format!("{}: {}", self.label, self.allowed());
        match self.kind {
            ValueKind::Integer { min, max } => {
                value.parse::<u64>().ok().filter(|n| (min..=max).contains(n)).map(|n| n.to_string()).ok_or_else(invalid)
            }
            ValueKind::Decimal { min, max } => {
                value.parse::<f64>().ok().filter(|n| n.is_finite() && (min..=max).contains(n)).map(|n| n.to_string()).ok_or_else(invalid)
            }
            ValueKind::Average { max } => {
                value.parse::<Average>().ok().filter(|n| n.scaled() <= max).map(|n| n.to_string()).ok_or_else(invalid)
            }
            ValueKind::Rate if value == "max" || value == "Maximum" => Ok("max".into()),
            ValueKind::Rate => {
                value.parse::<u64>().ok().filter(|n| (1..=1_000_000_000).contains(n)).map(|n| n.to_string()).ok_or_else(invalid)
            }
        }
    }
    pub fn allowed(&self) -> String {
        let number = crate::support::number;
        match self.kind {
            ValueKind::Integer { min, max } => format!("whole number {}–{} {}", number(min), number(max), self.unit),
            ValueKind::Decimal { min, max } => format!("{min}–{max} {}", self.unit),
            ValueKind::Average { max } => format!("1–{} snapshots/event; up to three decimal places", Average::from_scaled(max)),
            ValueKind::Rate => "Maximum or 1–1,000,000,000 events/s".into(),
        }
    }
}
