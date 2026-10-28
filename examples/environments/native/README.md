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

Client movement launched through the central launcher uses the shared
[Bevy presentation](../../support/graphics/README.md), with the portable workload
compiled natively. ConTime runs in the selected Runtime or Manual mode; only
queried scene reports cross into the graphics process. Direct terminal and
headless movement entry points remain available. Other cases retain terminal UI.
