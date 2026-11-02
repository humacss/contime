//! Startup metadata for the service health workload.
use super::{Definition, ValueKind};
use crate::support::uptime_config::Config;
pub fn definitions(c: &Config) -> Vec<Definition> {
    let mut definitions = super::contime(c.workers, c.manual, c.retention_ms, c.checkpoint_interval);
    if let Some(retention) = definitions.iter_mut().find(|definition| definition.id == "--retention-ms") {
        retention.kind = ValueKind::Integer { min: 1, max: (u64::MAX - 1) / 1_000_000 };
        retention.help = "Retained history for service health changes; delayed inputs hold advancement back until they are safe.";
    }
    for (id, label, help, unit, min, max, value, presets) in [
        (
            "--services",
            "Services",
            "Independent services whose health histories are reconciled.",
            "services",
            1,
            16,
            c.services as u64,
            vec![1, 3, 5, 10, 16],
        ),
        (
            "--cadence-ms",
            "Health change cadence",
            "Time between deterministic service status changes.",
            "ms",
            100,
            60000,
            c.cadence_ms,
            vec![100, 250, 500, 1000, 2000, 5000],
        ),
        (
            "--latency-ms",
            "Average delivery latency",
            "One-way delay of health changes before delivery.",
            "ms",
            0,
            (u64::MAX - 1) / 1_000_000,
            c.latency_ms,
            vec![0, 100, 500, 800, 1000, 2000, 5000],
        ),
        (
            "--jitter-ms",
            "Latency jitter",
            "Symmetric delivery variation, capped at the average latency.",
            "ms",
            0,
            (u64::MAX - 1) / 1_000_000,
            c.jitter_ms,
            vec![0, 100, 500, 800, 1000, 2000, 5000],
        ),
        (
            "--min-delay-ms",
            "Minimum delivery delay",
            "Lower bound applied after latency jitter; zero allows immediate delivery.",
            "ms",
            0,
            (u64::MAX - 1) / 1_000_000,
            c.min_delay_ms,
            vec![0, 25, 50, 100, 250, 500],
        ),
        (
            "--query-hz",
            "Queries / second",
            "Maximum completed health history updates per second.",
            "queries/s",
            1,
            60,
            c.query_hz,
            vec![1, 5, 10, 20, 30, 60],
        ),
    ] {
        definitions.push(Definition {
            id,
            group: "Service uptime",
            label,
            help,
            unit,
            kind: ValueKind::Integer { min, max },
            default: value.to_string(),
            choices: presets.iter().map(ToString::to_string).collect(),
        });
    }
    definitions
}
