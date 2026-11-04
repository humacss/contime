//! Setting explanations and editing. Common values are shortcuts, not restrictions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    SnapshotCount,
    Workers,
    Rate,
    Queries,
    Retention,
    Checkpoints,
    Pending,
    AverageTargets,
    MaxTargets,
    SnapshotSkew,
    Reorder,
}
impl Kind {
    pub const ALL: [Self; 11] = [
        Self::SnapshotCount,
        Self::Workers,
        Self::Rate,
        Self::Queries,
        Self::Retention,
        Self::Checkpoints,
        Self::Pending,
        Self::AverageTargets,
        Self::MaxTargets,
        Self::SnapshotSkew,
        Self::Reorder,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::SnapshotCount => "Snapshot count",
            Self::Workers => "Workers",
            Self::Rate => "Target event rate",
            Self::Queries => "Queries per second",
            Self::Retention => "History retention",
            Self::Checkpoints => "Checkpoint spacing",
            Self::Pending => "Pending source events",
            Self::AverageTargets => "Avg snapshots/event",
            Self::MaxTargets => "Max snapshots/event",
            Self::SnapshotSkew => "Snapshot skew",
            Self::Reorder => "Out-of-order delivery",
        }
    }
    pub fn flag(self) -> &'static str {
        match self {
            Self::SnapshotCount => "--snapshots",
            Self::Workers => "--workers",
            Self::Rate => "--events-per-second",
            Self::Queries => "--query-hz",
            Self::Retention => "--retention-ms",
            Self::Checkpoints => "--checkpoint-interval",
            Self::Pending => "--max-pending",
            Self::AverageTargets => "--average-snapshots-per-event",
            Self::MaxTargets => "--max-snapshots-per-event",
            Self::SnapshotSkew => "--snapshot-skew",
            Self::Reorder => "--reorder-percent",
        }
    }
    pub fn unit(self) -> &'static str {
        match self {
            Self::SnapshotCount => "snapshots",
            Self::Workers => "workers",
            Self::SnapshotSkew => "0–100",
            Self::Reorder => "%",
            Self::Rate => "events/s",
            Self::Queries => "queries/s",
            Self::Retention => "ms",
            Self::Checkpoints => "events",
            Self::Pending => "source events",
            Self::AverageTargets | Self::MaxTargets => "snapshots",
        }
    }
    pub fn bounds(self) -> (u64, u64) {
        (
            if self == Self::AverageTargets {
                1000
            } else if matches!(self, Self::SnapshotSkew | Self::Reorder) {
                0
            } else {
                1
            },
            match self {
                Self::SnapshotCount => 1_000_000,
                Self::Workers => 1024,
                Self::SnapshotSkew | Self::Reorder => 100,
                Self::Rate | Self::Checkpoints => 1_000_000_000,
                Self::Queries => 1000,
                Self::Retention => 86_400_000,
                Self::Pending => 10_000_000,
                Self::AverageTargets => 1_000_000_000,
                Self::MaxTargets => 1_000_000,
            },
        )
    }
    pub fn common(self) -> &'static [u64] {
        match self {
            Self::SnapshotCount => &[1, 2, 4, 8, 16, 64, 256, 1000, 10000, 100000, 1000000],
            Self::SnapshotSkew | Self::Reorder => &[0, 10, 25, 50, 75, 90, 99, 100],
            Self::Workers => &[1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024],
            Self::Rate => &[1, 10, 100, 1000, 10000, 100000, 1000000, 10000000, 100000000, 1000000000],
            Self::Queries => &[1, 5, 10, 20, 30, 60, 120, 240, 500, 1000],
            Self::Retention => &[1, 10, 100, 1000, 5000, 10000, 30000, 60000, 300000, 3600000, 86400000],
            Self::Checkpoints => &[1, 8, 16, 32, 64, 128, 256, 1000, 10000, 100000, 1000000, 1000000000],
            Self::Pending => &[1, 16, 64, 256, 1024, 4096, 16384, 65536, 262144, 1000000, 10000000],
            Self::AverageTargets => &[1000, 1100, 1500, 2000, 4000, 8000, 16000, 64000, 256000, 1000000, 10000000, 100000000, 1000000000],
            Self::MaxTargets => &[1, 2, 4, 8, 16, 64, 256, 1000, 10000, 100000, 1000000],
        }
    }
    pub fn explanation(self) -> &'static str {
        match self {
            Self::SnapshotCount => "Choose how many independent hash histories the benchmark can update. More histories can spread work across workers, but also use more memory.",
            Self::Workers => "Choose how many background worker threads process histories. More workers can help when there is enough independent work, but they also add overhead.",
            Self::Rate => "Choose how quickly events are generated. Maximum adjusts the offered load using worker progress; a number sets a fixed total rate across workers.",
            Self::Queries => "Set the maximum number of verification queries per second. Checks run at completed work boundaries and compare every hash with its expected history.",
            Self::Retention => "Choose how long past events remain available for replay and queries. A longer window retains more history and generally uses more memory.",
            Self::Checkpoints => "Choose approximately how many events separate saved states in each history. More frequent checkpoints use more memory but can reduce replay work.",
            Self::Pending => "For a fixed event rate, limit how many submitted events can wait for worker confirmation. A larger limit allows more backlog and memory use.",
            Self::AverageTargets => "Choose the average number of different histories each event affects. A value above one sends some events to multiple histories; it cannot exceed the maximum.",
            Self::MaxTargets => "Limit how many different histories one event can affect. The limit cannot exceed the snapshot count and must cover the chosen average.",
            Self::Reorder => "Reverse this percentage of each event group. Zero keeps events in order; 100 reverses the whole group. Small groups may reverse at least two events.",
            Self::SnapshotSkew => "Control how unevenly events are distributed. Zero spreads them evenly on average; higher values concentrate work on fewer histories.",
        }
    }
}

use super::{Definition, ValueKind};
use crate::support::Average;
pub fn common(settings: &crate::support::Settings, manual: bool) -> Vec<Definition> {
    let values = [
        settings.snapshot_count as u64,
        settings.workers as u64,
        settings.event_rate.form_value(),
        settings.query_hz as u64,
        settings.retention_ms,
        settings.checkpoint_interval as u64,
        settings.max_pending as u64,
        settings.average_targets.scaled(),
        settings.max_targets as u64,
        settings.snapshot_skew as u64,
        settings.reorder_percent as u64,
    ];
    // Group order is explicit; no caller depends on display labels or row indices.
    [
        Kind::Workers,
        Kind::Retention,
        Kind::Checkpoints,
        Kind::SnapshotCount,
        Kind::Rate,
        Kind::Reorder,
        Kind::Queries,
        Kind::AverageTargets,
        Kind::MaxTargets,
        Kind::SnapshotSkew,
        Kind::Pending,
    ]
    .into_iter()
    .filter(|kind| !manual || *kind != Kind::Workers)
    .map(|kind| {
        let value = values[Kind::ALL.iter().position(|k| *k == kind).expect("known setting")];
        definition(kind, value)
    })
    .collect()
}
pub fn contime(workers: usize, manual: bool, retention: u64, checkpoint: u64) -> Vec<Definition> {
    [(Kind::Workers, workers as u64), (Kind::Retention, retention), (Kind::Checkpoints, checkpoint)]
        .into_iter()
        .filter(|(kind, _)| !manual || *kind != Kind::Workers)
        .map(|(kind, value)| definition(kind, value))
        .collect()
}
fn definition(kind: Kind, value: u64) -> Definition {
    let (min, max) = kind.bounds();
    let mut choices: Vec<_> = kind
        .common()
        .iter()
        .map(|n| match kind {
            Kind::AverageTargets => Average::from_scaled(*n).to_string(),
            _ => n.to_string(),
        })
        .collect();
    if kind == Kind::Rate {
        choices.push("Maximum".into());
    }
    Definition {
        id: kind.flag(),
        group: if matches!(kind, Kind::Workers | Kind::Retention | Kind::Checkpoints) { "ConTime" } else { "Traffic and verification" },
        label: kind.name(),
        help: kind.explanation(),
        unit: kind.unit(),
        kind: match kind {
            Kind::Rate => ValueKind::Rate,
            Kind::AverageTargets => ValueKind::Average { max },
            _ => ValueKind::Integer { min, max },
        },
        choices,
        default: if kind == Kind::Rate && value == 0 {
            "Maximum".into()
        } else if kind == Kind::AverageTargets {
            Average::from_scaled(value).to_string()
        } else {
            value.to_string()
        },
    }
}
