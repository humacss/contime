# Live WebAssembly host

Both counter and chain-hash workloads use the same simulator callbacks, Maximum controller, worker-status
collection and final reference validation as native Runtime and Manual counters.
All load generation, ConTime processing, retention and validation run inside the
WebAssembly guest. The launcher renders the shared worker-monitor dashboard from
reported metrics; it does not produce or apply events.

Standard WASI uses one caller-driven Manual worker. Threaded WASI supports Manual
or Runtime, with configurable Runtime worker count. Startup settings are fixed
before processing starts. Maximum uses bounded production windows and worker
reports to adapt offered load independently of query frequency. A numeric rate
paces events instead. Both modes validate every snapshot before a timed run ends. Chain-hash reverses arrivals within each production group and validates periodically at the selected query frequency, freezing advancement while each full-state query completes so short retention remains safe.

```sh
# From the repository root: select the environment, execution and settings.
cargo run --release --features monitor --example launcher
```

The terminal UI shows target source events/s, cumulative completed source events/s,
unconfirmed events, and each worker's cumulative inserted/replayed application
totals and average rates. Arrow/page keys scroll workers. Repeated monitor readings
do not reset the averaging baseline. q / Escape or Ctrl-C stops the guest; timed
runs emit a final record only after monitor-confirmed drain and validation.

Direct or headless runs print text once per second and continue until stopped,
unless `--duration SECONDS` is supplied. A final `DRAINED (validated)` report means
all submitted events were confirmed and all snapshots matched the reference.
The standalone guest flushes output and exits without retained-history teardown;
the WASI host reclaims its memory. Browser sessions use their existing host.

Run commands below from this example directory so its target configuration is
loaded. This crate is excluded from the parent workspace.

## Standard WASI

```sh
rustup target add wasm32-wasip1 --toolchain 1.90.0
cargo +1.90.0 build --target wasm32-wasip1 --release
node run.mjs --snapshots 8 --events-per-second 100
```

`run.mjs` supplies standard WASI imports and forwards arguments. It contains no
load generation or application model. Standard WASI supports Manual execution.
No browser, DOM API or JavaScript clock is needed.

## Threaded WASI

Use Wasmtime 36.x LTS (tested with 36.0.19), whose WASI threads interface supports
the Rust target used here. Newer Wasmtime releases removed this interface.

```sh
rustup target add wasm32-wasip1-threads --toolchain 1.90.0
cargo +1.90.0 build --release --target wasm32-wasip1-threads --features runtime
wasmtime run --wasi threads=y target/wasm32-wasip1-threads/release/contime-wasm-example.wasm --execution runtime --workers 4 --snapshots 8 --events-per-second 100 --duration 5
```

Manual also runs on this threaded target. Runtime defaults to the available CPU
count reported by the host; `--workers` selects its worker count. Rust 1.90 needs
an explicit shared-memory growth maximum. The target configuration supplies a
1 GiB ceiling; it does not allocate 1 GiB at startup.

## Workload settings

`--help` lists the full shared settings: snapshot count, worker count (Runtime),
event rate (`max` or a positive integer), average/maximum snapshots per event,
query frequency, retention, checkpoint spacing, a pending-work allowance for numeric rates and an
optional duration. Manual accepts only one worker. `--no-setup` and `--headless`
are accepted for launcher compatibility. Direct runs use text output.
The launcher uses internal `--metrics-stdio` output for display metrics and
worker reports. The versioned worker protocol uses typed integer counts, retains
unknown/saturated values, and limits each record to 1 MiB. The launcher keeps a
bounded two-record queue, drains diagnostics and reaps the guest on completion
or error. A successful final record also requires a successful process exit.

`--domain counter` is the default. `--domain chain-hash` selects the shared hash
model with the same live load. Finite verification flags (`--rounds` and
`--events-per-round`) are no longer accepted by the live command.

The counter model and setup live in [`../../cases/counter`](../../cases/counter); simulator
callbacks remain in `counter/monitored/generation.rs`. Shared
[`../../support/monitored`](../../support/monitored/README.md) owns the simulator loop,
worker feedback, execution adapters and validation. `../../support/composition/wasi/workload.rs` supplies
WASI output, and `../../support/composition/wasi/support.rs` composes modules without terminal dependencies.
The hash model and simulator callbacks live under `cases/chain_hash/`. Both domains use the same versioned worker protocol; bounded snapshot previews include complete 32-byte hashes. Worker monitoring and application counters are always enabled, including builds without default features.
No ConTime execution code is changed.
The older controlled `Scenario` remains exported for portable regression tests.

```sh
cargo +1.90.0 test --features runtime
cargo +1.90.0 clippy --all-targets --features runtime -- -D warnings
```

These runs include example generation and correctness checks; their totals are
live diagnostics rather than isolated engine throughput measurements.

## Hash snapshot preview

The launcher shows the shared worker monitor for both domains. A bounded preview
reports produced and verified snapshot counts, values and full hash digests;
validation still compares every configured snapshot. Direct text output includes
up to eight verified hashes, with the final preview emitted after final validation.

## Snapshot traffic distribution

Set **Snapshot skew** in the terminal startup form, or pass
`--snapshot-skew 90` after the launcher's `--`. The browser uses the same setting.
The range is 0–100, default 0. Zero preserves the existing even distribution;
higher values favor lower-numbered snapshots. Try 50, 90 or 99 to make some
snapshots increasingly hot. At 100, events target the lowest snapshot IDs needed
for their distinct targets. The average and maximum snapshots per event remain
unchanged. Skew controls traffic concentration, not a percentage of active
snapshots or a guaranteed share of events. Manual and Runtime use the same
repeatable selection, independent of timing and backpressure.

## Client movement

Every live case is available here: `counter`, `chain-hash`, and `client-movement`.
The launcher opens the same Bevy movement window as Native, while the guest runs
the simulator, delayed network, monitor and query validation. WASD/arrows choose
a persistent direction; Space stops; q/Esc drains and validates before exit.
Standard WASI uses Manual; WASI Threads supports Manual and Runtime.

```sh
cargo run --release --features monitor --example launcher -- --environment wasm --example client-movement
cargo run --release --features monitor --example launcher -- --environment wasm-threads --example client-movement --execution runtime
```

Guest control input is a private stdin pipe, enabled by the launcher. Node uses
nonblocking reads; Wasmtime 36 uses zero-timeout readiness polling because its
legacy threaded host does not grant stdin descriptor-flag changes.
No input thread or blocking keyboard wait runs in single-threaded WebAssembly.
After building `wasm32-wasip1`, exercise the real Node input transport with:

```sh
node --test tests/movement.mjs
```

Client movement presentation uses the isolated
[Bevy example](../../support/graphics/README.md), requiring Rust 1.95 for the
native graphics window. ConTime still runs inside the selected WASI guest with
its existing Rust 1.90 baseline. Other cases retain the terminal dashboard.
