# Native environment

`host.rs` owns native process cancellation and terminal lifetime. It runs an
injected callback and has no case registry, event generation, monitoring or
validation policy. Shared `../execution.rs` starts Runtime or Manual ConTime
instances from execution configuration and supplied event/snapshot contracts.

Case-specific Cargo entry points live in
[`../../support/entrypoints`](../../support/entrypoints/README.md).
The support host attaches simulation, monitoring, validation and presentation.
Runtime starts background threads; Manual processes one complete operation per
shared host turn.
