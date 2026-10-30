//! Movement-specific settings plus the ConTime settings shared with other cases.
use super::{Definition, ValueKind};
use crate::support::{movement_config::Config, movement_report::MAX_CLIENTS};
pub fn definitions(c: &Config) -> Vec<Definition> {
    let mut definitions = super::contime(c.workers, c.manual, c.retention_ms, c.checkpoint_interval);
    if let Some(retention) = definitions.iter_mut().find(|d| d.id == "--retention-ms") {
        retention.help = "Automatically grows to cover delay + trail + 500 ms until explicitly edited. Longer windows use more memory.";
    }
    let mut add = |id, group, label, help, unit, min, max, value: u64, choices: &[u64]| {
        definitions.push(Definition {
            id,
            group,
            label,
            help,
            unit,
            kind: ValueKind::Integer { min, max },
            default: value.to_string(),
            choices: choices.iter().map(ToString::to_string).collect(),
        });
    };
    add(
        "--clients",
        "Client movement",
        "Clients (including you)",
        "One to five clients, including you. Client 1 uses WASD/arrows; other clients turn automatically.",
        "clients",
        1,
        MAX_CLIENTS as u64,
        c.clients as u64,
        &[1, 2, 3, 4, 5],
    );
    add(
        "--client-spacing",
        "Client movement",
        "Client spacing",
        "Distance between starting positions in world units. Zero starts clients together. Fixed for the run.",
        "units",
        0,
        1_000_000,
        c.client_spacing,
        &[0, 1, 2, 4, 8, 16, 32, 64, 128, 1000, 1_000_000],
    );
    add(
        "--movement-speed",
        "Client movement",
        "Movement speed",
        "Distance travelled each second while moving. Zero keeps clients stationary; input and replay validation still run.",
        "units/s",
        0,
        1_000_000,
        c.movement_speed,
        &[0, 1, 3, 6, 12, 24, 60, 120, 1000, 1_000_000],
    );
    add(
        "--input-hz",
        "Client movement",
        "Inputs / bot / second",
        "Direction changes per bot, evenly staggered across client slots. Keyboard inputs use the same simulated delivery queue.",
        "inputs/s",
        1,
        60,
        c.input_hz,
        &[1, 5, 10, 20, 60],
    );
    add(
        "--query-hz",
        "Client movement",
        "Queries / second",
        "Maximum completed rendering updates. Each update also queries its historical trail.",
        "queries/s",
        1,
        60,
        c.query_hz,
        &[1, 5, 10, 20, 30, 60],
    );
    add(
        "--average-latency-ms",
        "Client movement",
        "Average latency (ms)",
        "Average one-way delay, including keyboard input. 50 means 50 ms; 1000 means one second.",
        "ms",
        0,
        u64::MAX,
        c.average_latency_ms,
        &[0, 50, 100, 350, 1000, 2000, 5000, 10000, 30000, 60000, 300000],
    );
    add(
        "--jitter-ms",
        "Client movement",
        "Latency jitter (± ms)",
        "Symmetric variation around the average, capped at that average. A wide range makes reordered arrivals common.",
        "ms",
        0,
        u64::MAX,
        c.jitter_ms,
        &[0, 25, 50, 100, 300, 600, 1000, 5000],
    );
    add(
        "--trail-ms",
        "Client movement",
        "Trail duration (ms)",
        "Historical path to query. Retention also covers network delay plus 500 ms.",
        "ms",
        200,
        5000,
        c.trail_ms,
        &[200, 1000, 2000, 5000],
    );
    add(
        "--fade-ms",
        "Client movement",
        "Old state fade (ms)",
        "How long old rendering states stay visible. At most 32 previous states are kept.",
        "ms",
        100,
        5000,
        c.fade_ms,
        &[100, 500, 1000, 2000, 5000],
    );
    definitions
}
