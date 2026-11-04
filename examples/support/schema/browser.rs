//! Browser frame allowance definitions.
use super::{Definition, ValueKind};
pub fn definitions(budget: f64, steps: usize) -> Vec<Definition> {
    vec![
        Definition {
            id: "--budget-ms",
            group: "Browser",
            label: "Processing budget / frame",
            help: "Set how much time the browser can spend processing in each frame. One operation can exceed this limit because it is not interrupted.",
            unit: "ms",
            kind: ValueKind::Decimal { min: 0.01, max: 1000.0 },
            choices: ["0.1", "0.5", "1", "2", "4", "8", "16", "1000"].map(str::to_owned).to_vec(),
            default: budget.to_string(),
        },
        Definition {
            id: "--max-steps",
            group: "Browser",
            label: "Processing steps / frame",
            help: "Limit how many processing steps the browser can perform in one frame. Processing also stops when the frame time budget is reached.",
            unit: "steps",
            kind: ValueKind::Integer { min: 1, max: 100_000 },
            choices: [1, 100, 1000, 10000, 100000].map(|n| n.to_string()).to_vec(),
            default: steps.to_string(),
        },
    ]
}
