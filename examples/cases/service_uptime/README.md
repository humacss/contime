# Service uptime

A delayed health report can change an outage that has apparently already ended.
For example, recovery at 12:05 arrives first, then a delayed report says the
service went down at 12:02. Its current status is healthy, but its history must
now include three minutes of downtime.

This case compares two views receiving exactly the same delivered reports:

- **With ConTime:** queried snapshots apply health changes at their original timestamps.
- **Without reconciliation:** the comparison changes status at delivery time.
  A late outage can incorrectly mark an already recovered service as down.

Each service has two aligned timelines: ConTime on top and arrival-order state
below, with icon-only cumulative time summaries beside them. The default service identities are soft cyan (#63CEDB), soft magenta (#E580BF), soft violet (#A28AE5),
light cocoa (#BE9478) and ivory (#D5D4C7);
green, amber and red belong only to health status. The shared action/delivery
timeline uses the same service colors. The renderer only displays workload reports;
it never generates or repairs state. Check marks mean healthy, exclamation marks mean degraded,
and X marks mean down. Delivered symbols animate into both histories, showing
which timestamp each view uses. Histories update when those symbols land,
with permanent status symbols marking transitions in each strip. Delivery stays at 65% of a fixed-width time window,
so long network delays do not rescale the display.

The default source cadence is exactly one second for every service. Delivery jitter
changes when those reports arrive, never their action timestamps. Both the input
timeline and the paired history display have an absolute one-second grid; history
grid lines span all service rows. Thus corrected transitions align to the grid,
while arrival-order transitions can fall between columns or in the wrong order.
A custom health-change cadence remains available through the launcher.
Upcoming health reports are generated 3.5 seconds ahead, filling the visible future
portion of the input timeline. They keep their original one-second action ticks
and independently sampled delivery times; only due reports enter ConTime.
Finite runs cap this preview at their configured duration.

## Run

From the repository root:

```sh
cargo run --example launcher
```

Choose **Visualizer**, an environment, and its supported execution mode, then
choose **Service uptime** in the graphical window.
Native and both WASI environments open the shared Bevy viewer. The browser uses
that same presentation compiled to WebAssembly. Browser processing keeps the
configured per-frame allowance. Rust 1.95 is needed for the example renderer;
ConTime and the workload still use Rust 1.90.

For a short validated run without a graphical window:

```sh
cargo run --all-features --example service_uptime -- --execution manual --headless --duration 8
cargo run --all-features --example service_uptime -- --execution runtime --workers 2 --headless --duration 8
```

The standalone WASI workload supports `--domain service-uptime`. The launcher
keeps that workload in the selected WASI environment, passing reports to the
desktop viewer; it does not replace it with native simulation.
See [browser prerequisites](../../environments/browser/README.md#build-directly)
and the [WASI instructions](../../environments/wasi/README.md).

## Definitions and settings

Start with [model.rs](model.rs): `StatusChanged` carries event identity, service
identity, the original timestamp and health status. `Service` accumulates status
intervals and durations. Derives supply time access, IDs, event registration and
creation; `SnapshotApply::apply_batch` owns the application rule.
[setup.rs](setup.rs) adapts those types to the shared execution interfaces.

Startup settings include service count, health-change cadence, delivery latency
and jitter, minimum delivery delay, query frequency, workers, retention and checkpoints. Settings stay
fixed during a run. Health phases vary between services. Every event gets an independent uniform random
delay within latency ± jitter (with jitter capped at latency), then the minimum
delay is applied. A fixed seed of 42 reproduces the network schedule across hosts
and resets; status and service identity do not select the delay. Out-of-order
arrivals can occur when delay variation exceeds the change cadence; they are not forced. The minimum delay defaults to 250 ms and clamps the jittered delay from below.
Adjust it with `--min-delay-ms` (or `min_delay_ms` in the browser URL). An explicit
zero minimum and zero latency remove delivery delay for comparison tests.

The workload keeps an independent chronological reference and checks each
completed ConTime query against it. It advances retention conservatively while
older reports are still in flight. Stop ends generation, discards future actions that have not occurred, and flushes remaining queued simulated reports immediately in their
scheduled delivery order, then drains ConTime and validates final queried state.
This avoids waiting for a long configured network delay when closing the viewer. The simulated network is bounded to 8,192 pending changes. Startup rejects
service-count, cadence and latency combinations that would exceed that capacity,
including look-ahead previews and a catch-up allowance. Short finite runs account for their duration.
The runtime guard remains for exceptional processing stalls. The timeline preview
contains at most 128 inputs, including up to 64 earliest pending deliveries and
the most recent arrivals in the remaining slots. Visible intervals are bounded; accumulated durations are retained separately.
There is no fixed run-duration limit beyond the application timestamp range.

Browser P and tab visibility pause the simulation clock, pending delivery and
query deadlines. Closing a desktop window requests a validated stop. Health
reports in this example are simulated; it is not a monitoring agent or a
replacement for persistent incident storage.
