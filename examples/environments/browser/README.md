# Browser live example

Counter, Chain hash and Client movement share the Manual backend and
`contime-monitor` worker readings with the terminal hosts. Counter and Chain hash
also use the shared `contime-simulator` callbacks and Maximum controller. One manual worker runs on the browser main thread; JavaScript
schedules frames and renders metrics. Rust owns event production, processing,
monitoring and retention. Chain hash reverses generated event groups after recording the canonical reference,
then validates full digests independently of production feedback.

## Launch

Run `cargo run --release --example launcher` from the repository root, then
choose Browser and Manual. Common startup settings match the terminal hosts:
snapshot count, event rate, average/maximum snapshots per event, query frequency,
retention, checkpoint interval and, for numeric rates, pending capacity. The browser additionally
asks for a wall-clock allowance and maximum shared steps per frame.
Configuration comes from the launcher; the browser has no settings panel. Direct
page loads use default settings, with optional startup URL parameters. Pause excludes paused time; Resume continues the same instance.
Reset clears the instance and metrics and retains that startup configuration.
Relaunch to change the configuration. Runs continue until paused or the page closes.

## Build directly

From this directory:

```sh
rustup target add wasm32-unknown-unknown --toolchain 1.90.0
cargo install wasm-bindgen-cli --version 0.2.127 --locked
cargo +1.90.0 build --target wasm32-unknown-unknown --release
wasm-bindgen target/wasm32-unknown-unknown/release/contime_browser_example.wasm --target web --out-dir ../../support/composition/browser/pkg
python3 -m http.server 8766 --bind 127.0.0.1 --directory ../../support/composition/browser
```

Open `http://127.0.0.1:8766`. No frontend dependency installation is needed.
Generated bindings, build outputs and lockfiles are ignored.

Each animation frame repeats complete host turns until its time allowance or
step limit is reached. Each turn performs at most one existing Manual
`process()` operation and one bounded simulator submission. There are no waits
or background threads. The allowance includes generation, processing, monitoring,
queries and pruning. It is soft: a complete operation may overrun it. Rendering
occurs afterward. Background tabs may receive fewer frames. Measurements include
example overhead and browser cadence.

For every case, **Confirmed events**, **Unconfirmed events** and **Avg completed/s**
refer to source events. The worker table shows cumulative inserted/replayed
snapshot applications and their cumulative average rates since the first worker
reading. One event can target multiple snapshots, and replay can visit an event
again. Unconfirmed events are submitted events minus monitor-confirmed windows;
they include work finished since the last confirmation. They are not an exact
queue length. Averages show a dash until a measuring interval is available.

Maximum adjusts bounded production windows using worker status, independently
of query frequency. Its pending setting is hidden; numeric rates retain a
configurable source-window ceiling. Counter queries run only after an explicit
stop and monitor-confirmed drain, then verify every snapshot against the generated
reference. The query-frequency setting applies to chain-hash validation.
Chain hash queries only at completed production-window boundaries, at most at
the configured frequency, and again after final drain. New submissions and
advancement pause while a query is pending so retention cannot discard its
captured target. Every snapshot count, value and full digest is checked; any
validation failure stops the run. The first 20 snapshot comparisons show the latest canonical
generated count and full digest with the last verified count and full digest.

## Automated live checks

Common URL parameters are `domain=counter|chain-hash`, `snapshots`, `rate` (`0`
means Maximum), `average_targets`, `max_targets`, `query_hz`, `retention_ms`,
`checkpoint_interval`, `max_pending` (numeric rates only), `max_ms` and `max_steps`.
Startup values are immutable; `autostart=1` starts after loading.

An optional positive `duration` in seconds stops generation and drains all work:

```text
http://127.0.0.1:8766/?domain=counter&snapshots=8&rate=1000&duration=3&autostart=1&max_ms=1&max_steps=16
```

There is no default duration or round count. Automation can read
`window.contimeExample`: `done` becomes true after an explicit duration drains,
`error` holds a failure, `report` holds live and per-frame metrics, and `frames`
counts calls. Reset clears these fields.

```sh
cargo +1.90.0 test
node --test ../../support/composition/browser/tests/pacing.mjs
```

Start with `src/scheduler.rs` for the case-agnostic frame budget.
`../../support/composition/browser/session.rs` selects a case, and its `frame.rs`
connects simulation, monitoring and validation. `src/clock.rs` supplies the browser clock;
`../../support/monitored` supplies the shared controller, backend and reference
checks. Counter simulation stays in `../../cases/counter/monitored/generation.rs` and
application definitions stay in `../../cases/counter/model.rs`. Presentation lives in
`../../support/composition/browser/ui`. The browser crate is independent of the WASI host and excluded from the
main workspace and publication.

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

The launcher also exposes **Client movement** with the same latency, jitter,
input cadence, retention and trail settings as Native/WASI. It uses the shared
movement driver, and the existing scheduler applies its processing time/step
allowance between complete turns. Click the map to send WASD/arrows and Space;
input is delayed and replayed in Rust before queried positions reach Canvas.
All clients are validated even when only a bounded camera-region preview is drawn.

```sh
cargo run --release --example launcher -- --environment browser --example client-movement
```
