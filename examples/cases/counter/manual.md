# Manual counters

The self-contained code sample in [`manual.rs`](manual.rs) submits two counters,
requests queries, and calls `process()` once per host-loop turn. It is included
in ConTime's Rust documentation rather than registered as a separate executable.
Use the shared live counter below to try Manual processing.

## Live simulator and monitor

The live counter uses the same simulator, Maximum controller and worker-monitor
UI as native Runtime counters. The execution adapter is the only difference:
Manual calls `process()` once per host-loop turn, on the calling thread.

```sh
cargo run --release --all-features --example counter -- --execution manual
cargo run --release --all-features --example counter -- --headless --duration 2 --snapshots 8 --events-per-second 1000 --execution manual
```

The launcher selects this same host when you choose Native / Manual / Counter:

```sh
cargo run --release --example launcher
```

Settings are fixed before processing starts. Manual uses one worker, so
`--workers` must be 1. Maximum adjusts offered load using actual worker reports,
independently of query frequency. Fixed numeric rates pace submissions instead.
The dashboard shows target events/s, average completed events/s, outstanding
unconfirmed events, and cumulative inserted/replayed totals and average rates
for the worker. Worker figures count target applications, not unique source events.

The simulator callbacks are in
[`monitored/generation.rs`](monitored/generation.rs).
They reuse [`model.rs`](model.rs) and
[`setup.rs`](setup.rs), including average/maximum targets
and snapshot traffic skew. Neither model nor callback knows the execution mode.
The common [monitored host](../../support/monitored/README.md) owns collection,
load control, reference validation and UI. Its `backend.rs` adapts execution.

Manual reserves queue slots for control/query messages and stops admitting new
batches while that queue fills. Each loop still performs one complete `process()`
step and returns to input, monitoring and rendering. It requests explicit
`prune_before` retention about every 100 ms through the existing pruning barrier.
Status inspection is read-only; it does not advance replay or process messages.
No background ConTime threads or resumable snapshot operations are introduced.

A duration limit, q / Escape, Ctrl-C or SIGTERM stops admission. The shared host
waits for monitor-confirmed completion, validates every counter against its
reference, restores the terminal, closes the backend and prints the summary.
Finite headless runs default to five seconds unless a duration is supplied.

Both execution modes are compiled together. `--execution manual` selects Manual;
`--execution runtime` selects the default Runtime mode. WASI counters also use this shared driver, with their workload inside WebAssembly
and terminal presentation in the launcher. Browser Manual counters share the same simulator, monitor and controller through a frame-budgeted adapter.
