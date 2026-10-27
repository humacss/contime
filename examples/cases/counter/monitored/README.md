# Counter simulator callbacks

`generation.rs` supplies the two callbacks used by `contime-simulator::EventStream`:
create a counter event with a unique ID and timestamp, then assign its distinct
snapshot destinations using the configured fanout and traffic skew.

The live counter entry point is
[`counter.rs`](../../../support/entrypoints/counter.rs). It combines the counter
model with the [shared monitored host](../../../support/monitored/README.md).
Runtime and Manual use the same generation, Maximum controller, worker monitor,
and final validation. WASI and browser compositions reuse these components too.

Run from the repository root:

```sh
cargo run --release --example launcher
cargo run --release --features monitor,counters --example counter_live
```

Startup settings are fixed for a run. These headless commands skip the form:

```sh
# Maximum throughput, two workers, shared events and concentrated traffic.
cargo run --release --features monitor,counters --example counter_live -- \
  --workers 2 --snapshots 1000 --average-snapshots-per-event 1.5 \
  --max-snapshots-per-event 3 --snapshot-skew 90 --headless --duration 5

# Fixed-rate comparison with the same monitor-confirmed completion feedback.
cargo run --release --features monitor,counters --example counter_live -- \
  --events-per-second 100000 --headless --duration 5
```

See [shared support](../../../support/monitored/README.md) for Maximum adaptation,
source-event versus snapshot-application counts, validation, and shutdown.
The [counter guide](../README.md) explains the event and snapshot model.

## Verification

```sh
cargo test --features manual,monitor,counters --example counter_live --example manual_counter_live
cargo clippy --features manual,monitor,counters --example counter_live --example manual_counter_live -- -D warnings
```

Tests preserve fixed/Maximum production, multiple workers and destinations,
Manual execution, short retention, unique event IDs and concentrated traffic.
Controller regressions remain in shared support.
