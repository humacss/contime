use crate::report::Report;
#[path = "../../format.rs"]
mod format;
use format::{decimal, number};

pub const COLUMNS: [f32; 3] = [-48.0, -16.0, 16.0];
pub const LABELS: [&str; 3] = ["Inputs arrived", "Reordered arrivals", "Corrections"];

/// Read-only strings derived from the same report on every graphical host.
pub struct Hud {
    pub settings: String,
    pub values: [String; 3],
    pub footer: String,
}
impl Hud {
    pub fn new(report: &Report, status: &str) -> Self {
        Self {
            settings: format!(
                "Clients {}   Workers {}   Avg lag {} +/- {} ms   Speed {}   Spacing {}",
                number(report.clients),
                number(report.workers),
                number(report.average_latency_ms),
                number(report.jitter_ms),
                number(report.movement_speed),
                number(report.client_spacing)
            ),
            values: [number(report.submitted), number(report.late), number(report.corrections)],
            footer: format!(
                "{status}   |   In flight {}   |   Query updates {}   |   Arena {} x {}   |   WASD / arrows, Space to stop",
                number(report.in_flight),
                number(report.updates),
                decimal(report.arena_half_extent[0] * 2.0, 0),
                decimal(report.arena_half_extent[1] * 2.0, 0)
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overlay_uses_read_only_settings_and_grouped_input_counters() {
        let mut report = crate::animation::tests::scene(0.0);
        report.clients = 16;
        report.average_latency_ms = 600;
        report.jitter_ms = 200;
        report.submitted = 1_234_567;
        report.late = 2_345;
        report.corrections = 3_456;
        report.in_flight = 123;
        report.updates = 4_567;
        let before = serde_json::to_string(&report).unwrap();
        let hud = Hud::new(&report, "Live");
        assert!(hud.settings.contains("Clients 16") && hud.settings.contains("Workers 1"));
        assert!(hud.settings.contains("600 +/- 200 ms"));
        assert_eq!(hud.values, ["1,234,567", "2,345", "3,456"]);
        assert!(hud.footer.contains("In flight 123") && hud.footer.contains("Query updates 4,567"));
        assert_eq!(serde_json::to_string(&report).unwrap(), before);
    }
    #[test]
    fn large_counts_remain_exact_and_do_not_move_overlay_columns() {
        let mut report = crate::animation::tests::scene(0.0);
        report.submitted = u64::MAX;
        assert_eq!(Hud::new(&report, "Paused").values[0], "18,446,744,073,709,551,615");
        assert_eq!(COLUMNS, [-48.0, -16.0, 16.0]);
    }
}
