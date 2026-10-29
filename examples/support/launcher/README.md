# Central example launcher

From the repository root:

```sh
cargo run --release --features monitor --example launcher
```

Runtime is enabled by default. Enable `monitor` for the launcher dashboard; the
launcher selects simulation and Manual support for its native child commands
automatically. Choosing an execution mode controls which backend is used; it
does not start the other backend. Portable hosts select their constrained build
features automatically. Monitoring includes worker application bookkeeping.

Choose an environment, execution mode, example, and live settings. One grouped startup form combines the applicable definitions:

- **ConTime:** worker count, history retention, checkpoint spacing. Manual uses
  one worker and hides its control.
- **Traffic and verification:** snapshot count, target distribution, event rate,
  query frequency, and pending work for numeric rates, for Counter and Chain hash.
- **Client movement:** clients, spacing, movement speed, input/query frequency, average latency, jitter,
  trail and fading, instead of throughput/fanout settings.
- **Browser:** frame processing time and step limits, shown only for that host.

Each field shows its allowed range, unit, help and numbered presets. Custom
numeric values are supported; presets are shortcuts, not restrictions. Long
forms scroll to keep the selected setting visible. Settings are fixed once the
example starts.

Use ↑/↓ or Tab to select a setting, ←/→ to change it, or type a numeric value.
The first choice is the default. Execution follows the environment default until
you explicitly select it; explicit choices are retained when supported and
restored when you return to a compatible environment.
Enter continues or starts; Esc cancels; `r` restores the initial value. Help and
common values appear for the selected field. Maximum event rate follows the
highest numeric preset, and `m` selects it directly.

| Environment | Execution | Examples |
| --- | --- | --- |
| Native terminal | Runtime or Manual | Counter, client movement, chain hash |
| Standalone WASI (`wasm32-wasip1`, Node) | Manual | Counter, client movement, chain hash |
| Standalone WASI threads (`wasm32-wasip1-threads`, Wasmtime 36.x LTS) | Runtime or Manual | Counter, client movement, chain hash |
| Browser | Manual | Counter, client movement, chain hash |

Counter event and snapshot definitions are shared across all four environments. Runs
continue until stopped unless a duration is supplied. Application definitions
stay under `examples/cases/counter/`, `examples/cases/client_movement/`, and
`examples/cases/chain_hash/`; the launcher
contains no domain logic. The ConTime library is unchanged by host selection.

All live native, WASI and browser cases use simulator callbacks and worker-monitor
feedback. The terminal dashboard shows cumulative inserted/replayed totals and cumulative
average rates per worker. Counter and hash Maximum production uses worker status
to adjust offered load; movement uses a configured per-client input cadence.
The launcher enables the required monitoring features automatically.
Hash and movement hosts also inspect and validate snapshot state. WASI hash
comparisons are a labelled preview of at most 64 snapshots with full digests;
the guest validates the complete population.
For WASI, the launcher owns terminal drawing and keyboard input; the worker and
live workload remain inside WebAssembly. Press q, Esc or Ctrl-C to stop the
WASI host. Explicit-duration runs drain their work and shut down normally.
Headless and redirected runs keep their human-readable text output.

Counter/hash settings are collected before launch and forwarded to the selected host.
Movement is configured in that same form and starts without a second prompt.
Its retention grows with latency until explicitly edited; explicit values are
validated against the delay and trail. Worker-dependent workload defaults follow
worker edits until their own values are explicitly set.
Browser settings are passed in its URL, start automatically, and remain fixed
through pause, restart, and completion. Relaunch to change them.

The browser launcher builds WebAssembly and bindings, serves assets on
`127.0.0.1` with an available port, and opens the browser. Ctrl-C stops the server.
Closing the tab leaves the server running. `--no-open` prints the URL without
requesting a browser window. Only browser assets are served.

## Direct and repeatable launches

`--list` shows supported combinations. `--help` describes launcher options.
`--dry-run` prints the launch plan without building or starting it and skips
interactive setup. `--no-setup` uses supplied settings and defaults directly.

Common example settings follow the final `--` for every environment:

```sh
cargo run --features monitor --example launcher -- --environment browser --example counter --dry-run
cargo run --release --features monitor --example launcher -- --environment native --execution manual --no-setup -- --headless --duration 2 --snapshots 8 --events-per-second 1000
cargo run --release --features monitor --example launcher -- --environment wasm --example counter --no-setup -- --snapshots 8 --events-per-second 1000
cargo run --release --features monitor --example launcher -- --environment wasm-threads --execution runtime --no-setup -- --workers 4 --snapshots 64 --duration 2
cargo run --release --features monitor --example launcher -- --environment browser --no-setup --budget-ms 1 --max-steps 1000 -- --snapshots 8 --events-per-second max --query-hz 20
```

`terminal` is an alias for `native`. The environment selects its build target
and host. `--workers N` and `--rate max|N` before the separator are convenience
aliases for common worker count and event rate. Manual requires one worker.
Browser frame budget ranges from 0.01 to 1,000 ms; steps range from 1 to 100,000.
The budget is soft because a single complete operation can exceed it.

Common flags include `--snapshots`, `--workers`,
`--average-snapshots-per-event`, `--max-snapshots-per-event`,
`--events-per-second`, `--query-hz`, `--retention-ms`, `--checkpoint-interval`,
`--max-pending`, and optional `--duration SECONDS`. Maximum event production
retains processing and pending-work limits. Native counter/hash `--headless` runs
default to five seconds when no duration is supplied. Movement runs until stopped
unless a duration is supplied. Portable hosts need no headless flag to run
without a terminal.

## Tools

Native launches build the existing example with both execution features.
Portable builds use Rust 1.90.0 and run from each host directory so its Cargo
configuration applies:

```sh
rustup target add wasm32-wasip1 wasm32-unknown-unknown --toolchain 1.90.0
```

Standard WASI requires Node.js with WASI support. Browser binding generation
requires `wasm-bindgen-cli` on PATH or selected by `WASM_BINDGEN`. The launcher
checks that it runs; wasm-bindgen checks compatibility with the compiled module.
Missing tools are reported and never installed automatically.

Threaded WASI requires **Wasmtime 36.x LTS**, tested with 36.0.19. The launcher
checks compatibility before building. Set `WASMTIME` when it is not on PATH;
relative explicit paths are resolved before changing directories.

```sh
rustup target add wasm32-wasip1-threads --toolchain 1.90.0
WASMTIME=/path/to/wasmtime-36 cargo run --release --features monitor --example launcher
```

The standalone example's threaded target configuration sets a 1 GiB maximum
shared-memory size for Rust 1.90's linker. It permits heap and thread-stack
growth without allocating the maximum at startup.

The launcher entry point is `main.rs` in this folder. `selection.rs` validates
capabilities, `menu.rs` presents host selection, `settings_form.rs` composes and validates
startup settings from `support/schema/`,
`launch.rs` dispatches child hosts, and `server.rs` manages browser serving.

## Snapshot table

The interactive terminal dashboard keeps aggregate statistics above a scrollable
snapshot grid. Snapshot/value column pairs fill the terminal width, with
borders between cells and right-aligned numbers. Use Up/Down to scroll one grid
row, Page Up/Page Down to move one screen, and Home/End to reach the first or last snapshots. The table heading
shows the visible range and total snapshot count. All configured snapshots are
available in native and both WASI environments; rates cover all snapshots and
are averaged since startup. Table values come from the latest completed query.
Headless output and the final printed summary keep a short preview.

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

## Shared settings definitions

`support/schema/` holds UI-independent definitions: stable flag IDs, grouping,
labels/help, units, value types, bounds, presets and initial values. The launcher
keeps selected values and editing state separately. It composes definitions for
the chosen case and host, then delegates cross-setting validation to the same
configuration parsers used by the examples. No case is selected by its display
label inside the settings form. `form.rs` only handles navigation and drawing.
Direct movement startup uses the same schema and grouped editor.

## Client movement in portable hosts

Select **Client movement** with any environment. The same shared driver generates
late inputs, checks monitor-confirmed historical queries, validates every client,
and publishes corrected rendering states. All graphical hosts share the isolated
[Bevy presentation](../graphics/README.md), requiring Rust 1.95 for graphics while
ConTime remains on Rust 1.90. WASI keeps simulation and validation in WebAssembly
and receives keyboard commands through an input pipe. Browser uses the same
renderer compiled to WebAssembly and retains its per-frame processing allowance. Click the map before using WASD,
arrows or Space. Relaunch to change settings.

```sh
cargo run --release --features monitor --example launcher -- --environment browser --example client-movement
cargo run --release --features monitor --example launcher -- --environment wasm --example client-movement
cargo run --release --features monitor --example launcher -- --environment wasm-threads --execution runtime --example client-movement
```
