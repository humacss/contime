# Central example launcher

From the repository root:

```sh
cargo run --example launcher
```

No feature flags are needed. The launcher enables its dashboard and the selected
workload's build features automatically. The startup form chooses which backend
actually runs; both execution modes, monitoring, and simulation are available to
native workloads. Portable hosts keep their platform-specific build configuration,
including Manual-only processing in a browser. Library defaults remain Runtime only.
Add `--release` when you want an optimized launcher build.

Choose an example first, then an environment. The selected option is explained below the choices. The selected example shows its overview from the Game or Benchmark README. Use Page Up and Page Down to scroll if needed. Benchmark measures throughput while verifying order-sensitive hash chains. **Game** is selected by default and starts
immediately with fixed demonstration settings. Native and WASI Threads use Runtime;
standard WASI and Browser use Manual. Benchmark offers an execution
mode choice within its workload settings.

Each field explains its purpose. Benchmark setting descriptions come from the
[Benchmark README](../../cases/chain_hash/README.md#benchmark-settings), which also
documents their command-line options. Custom numeric values are supported; presets
are shortcuts, not restrictions. Invalid values are explained when you start the run. Long
forms scroll to keep the selected setting visible. Settings are fixed once the
example starts.

Use ↑/↓ or Tab to select a setting, ←/→ to change it, or type a numeric value.
The first choice is the default. Benchmark uses the environment’s
default execution mode until you choose another in its configuration.
Enter continues or starts; Esc cancels; `r` restores the initial value. Help and
the selected setting’s description appears below the form. Maximum event rate follows the
highest numeric preset, and `m` selects it directly.

| Environment | Execution | Examples |
| --- | --- | --- |
| Native | Runtime or Manual | Game, Benchmark |
| WebAssembly / Node (WASI) | Manual | Game, Benchmark |
| WebAssembly / Wasmtime threads | Runtime or Manual | Game, Benchmark |
| WebAssembly / Browser | Manual | Game, Benchmark |

Benchmark event and snapshot definitions are shared across all four environments. Runs
continue until stopped unless a duration is supplied. Application definitions
stay under `examples/cases/client_movement/` and
`examples/cases/chain_hash/`; the launcher
contains no domain logic. The `contime` library is unchanged by host selection.

All live native, WASI and browser cases use simulator callbacks and worker-monitor
feedback. The same Bevy presentation displays the movement comparison or the
hash metrics in every environment. Benchmark Maximum production
uses worker status to adjust offered load; movement uses a configured per-client
input cadence. Hash validation covers all snapshots. The graphical dashboard shows
throughput and worker activity. Reset restarts the workload after validation.
Headless and direct terminal runs keep their text output.

Benchmark settings are collected before launch and forwarded to the selected host.
Execution, workers, snapshot count, event rate, out-of-order delivery, query frequency and retention are
visible by default. The **Advanced** section is also shown by default, with checkpoint
spacing, fanout, traffic skew, backlog and browser processing limits. Use **Advanced
settings** to hide or show it. Hiding the section preserves its values, and the same
options remain available on the command line.
Game starts without a settings form. Worker-dependent defaults for Benchmark
follow worker edits until their values are explicitly set.
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

For Benchmark, common example settings follow the final `--`:

```sh
cargo run --example launcher -- --environment browser --example benchmark --dry-run
cargo run --release --example launcher -- --environment native --example benchmark --execution manual --no-setup -- --headless --duration 2 --snapshots 8 --events-per-second 1000
cargo run --release --example launcher -- --environment wasm --example benchmark --no-setup -- --snapshots 8 --events-per-second 1000
cargo run --release --example launcher -- --environment wasm-threads --example benchmark --execution runtime --no-setup -- --workers 4 --snapshots 64 --duration 2
cargo run --release --example launcher -- --environment browser --example benchmark --no-setup --budget-ms 1 --max-steps 1000 -- --snapshots 8 --events-per-second max --query-hz 20
```

`terminal` is an alias for `native`. The environment selects its build target
and host. `--workers N` and `--rate max|N` before the separator are convenience
aliases for common worker count and event rate. Manual requires one worker.
Browser frame budget ranges from 0.01 to 1,000 ms; steps range from 1 to 100,000.
The budget is soft because a single complete operation can exceed it.

Common flags include `--snapshots`, `--workers`,
`--average-snapshots-per-event`, `--max-snapshots-per-event`,
`--events-per-second`, `--reorder-percent`, `--query-hz`, `--retention-ms`, `--checkpoint-interval`,
`--max-pending`, and optional `--duration SECONDS`. Maximum event production
retains processing and pending-work limits. Native chain hash `--headless` runs
default to five seconds when no duration is supplied. Game runs until stopped. Portable hosts need no headless flag to run
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
WASMTIME=/path/to/wasmtime-36 cargo run --release --example launcher
```

The standalone example's threaded target configuration sets a 1 GiB maximum
shared-memory size for Rust 1.90's linker. It permits heap and thread-stack
growth without allocating the maximum at startup.

The launcher entry point is `main.rs` in this folder. `selection.rs` validates
capabilities, `menu.rs` presents host selection, `settings_form.rs` composes and validates
startup settings from `support/schema/`,
`launch.rs` dispatches child hosts, and `server.rs` manages browser serving.

## Worker diagnostics

The graphical Benchmark view shows a throughput history and worker
activity. Use the arrow keys or Page Up and Page Down to browse workers. The
workload validates all snapshots.

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

Select **Game** with any environment. The same shared driver generates
late inputs, checks monitor-confirmed historical queries, validates every client,
and publishes corrected rendering states. All graphical hosts share the isolated
[Bevy presentation](../graphics/README.md), requiring Rust 1.95 for graphics while
`contime` remains on Rust 1.90. WASI keeps simulation and validation in WebAssembly
and receives keyboard commands through an input pipe. Browser uses the same
renderer compiled to WebAssembly and retains its per-frame processing allowance. Click the map before using WASD,
arrows or Space. The launcher uses the same default game settings for each run.

```sh
cargo run --release --example launcher -- --environment browser --example game
cargo run --release --example launcher -- --environment wasm --example game
cargo run --release --example launcher -- --environment wasm-threads --example game
```
