# Browser live example

<!-- launcher:browser:start -->
Browser runs the workload and the shared graphical display as WebAssembly in your browser. The launcher builds the files, serves them locally and opens a page. Computation remains in the browser and no application server is required.

A single Manual worker processes complete operations within a configurable per-frame time allowance and step limit. Rendering and browser scheduling share that environment, so frame cadence and background-tab throttling can affect throughput.

The workload pauses when the tab is hidden. Press P to pause or resume manually, and use Reset to restart with the same settings. Building requires the WebAssembly target and a compatible wasm-bindgen tool.
<!-- launcher:browser:end -->

Chain hash and Client movement share the Manual backend and
`contime-monitor` worker readings with the terminal hosts. Chain hash
also uses the shared `contime-simulator` callbacks and Maximum controller. One manual worker runs on the browser main thread; JavaScript
schedules workload frames; the shared Bevy renderer draws both live examples. Rust owns event production, processing,
monitoring and retention. Chain hash records a canonical reference before reordering the configured fraction of each event group,
then validates full digests independently of production feedback.

## Launch

Run `cargo run --release --example launcher` from the repository root, then
choose WebAssembly / Browser. The browser uses Manual processing. Common startup settings match the terminal hosts:
snapshot count, event rate, average/maximum snapshots per event, query frequency,
retention, checkpoint interval and, for numeric rates, pending capacity. The browser additionally
asks for a wall-clock allowance and maximum shared steps per frame.
Configuration comes from the launcher; the browser has no settings panel. Direct
page loads use default settings, with optional startup URL parameters. Each case starts
automatically and displays only the shared graphical view. Reset clears the instance
and retains the startup configuration; P pauses and resumes processing.
Relaunch to change the configuration. Runs continue until paused or the page closes.

## Build directly

From this directory (Rust 1.95 is isolated to the Bevy presentation):

```sh
rustup target add wasm32-unknown-unknown --toolchain 1.90.0
cargo install wasm-bindgen-cli --version 0.2.127 --locked
cargo +1.90.0 build --target wasm32-unknown-unknown --release
wasm-bindgen target/wasm32-unknown-unknown/release/contime_browser_example.wasm --target web --out-dir ../../support/composition/browser/pkg
rustup toolchain install 1.95.0 --profile minimal --target wasm32-unknown-unknown
cargo +1.95.0 build --manifest-path ../../support/graphics/Cargo.toml --release --lib --target wasm32-unknown-unknown
wasm-bindgen ../../support/graphics/target/wasm32-unknown-unknown/release/contime_graphics_example.wasm --target web --out-dir ../../support/composition/browser/pkg/graphics
python3 -m http.server 8766 --bind 127.0.0.1 --directory ../../support/composition/browser
```

Open `http://127.0.0.1:8766`. No JavaScript package installation is needed. Client movement uses the isolated
[Bevy presentation](../../support/graphics/README.md).
Generated bindings, build outputs and lockfiles are ignored.

Each animation frame repeats complete host turns until its time allowance or
step limit is reached. Each turn performs at most one existing Manual
`process()` operation and one bounded simulator submission. There are no waits
or background threads. The allowance includes generation, processing, monitoring,
queries and pruning. It is soft: a complete operation may overrun it. Rendering
occurs afterward. Background tabs may receive fewer frames. Measurements include
example overhead and browser cadence.

For the hash case, the shared display shows produced, confirmed and
outstanding source events, query count, snapshot count, workers and average
completed events per second. Outstanding includes work awaiting confirmation and
is not an exact queue length. One event can target multiple snapshots, and replay
can visit an event again.

Maximum adjusts bounded production windows using worker status, independently
of query frequency. Its pending setting is hidden; numeric rates retain a
configurable source-window ceiling. The query-frequency setting applies to chain-hash validation.
Chain hash queries only at completed production-window boundaries, at most at
the configured frequency, and again after final drain. New submissions and
advancement pause while a query is pending so retention cannot discard its
captured target. Every snapshot count, value and full digest is checked; any
validation failure stops the run. The graphical dashboard shows throughput and
worker activity, and the complete population is still validated.

## Pause and resume

Hiding the tab automatically pauses processing and freezes active simulation time.
Returning resumes the run without catching up the time spent away. In every case,
press P to pause or resume manually; Space still stops the movement character. A manual pause
survives tab changes. The shared game view shows Paused, including while animations
are frozen. Pending events and queries are preserved; their simulated deadlines do
not include paused wall-clock time. Completed or failed runs do not resume automatically.

## Automated live checks

Common URL parameters are `domain=chain-hash|client-movement`, `snapshots`, `rate` (`0`
means Maximum), `average_targets`, `max_targets`, `query_hz`, `retention_ms`,
`checkpoint_interval`, `reorder_percent`, `max_pending` (numeric rates only), `max_ms` and `max_steps`.
Startup values are immutable and the example starts after loading.

An optional positive `duration` in seconds stops generation and drains all work:

```text
http://127.0.0.1:8766/?domain=chain-hash&snapshots=8&rate=1000&duration=3&autostart=1&max_ms=1&max_steps=16
```

There is no default duration or round count. Automation can read
`window.contimeExample`: `done` becomes true after an explicit duration drains,
`error` holds a failure, `report` holds live and per-frame metrics, and `frames`
counts calls. Reset clears these fields.

```sh
cargo +1.90.0 test
node --test ../../support/composition/browser/tests/*.mjs
```

Start with `src/scheduler.rs` for the case-agnostic frame budget.
`../../support/composition/browser/session.rs` selects a case, and its `frame.rs`
connects simulation, monitoring and validation. `src/clock.rs` supplies the browser clock;
`../../support/monitored` supplies the shared controller, backend and reference
checks. Hash simulation and application definitions stay under `../../cases/chain_hash/`. Presentation lives in
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

The launcher exposes **Game** with the same fixed demonstration settings as
Native and WASI. It uses the shared
movement driver, and the existing scheduler applies its processing time/step
allowance between complete turns. Click the map to send WASD/arrows and Space;
input is delayed and replayed in Rust before queried positions reach Bevy.
All clients are validated even when only a bounded camera-region preview is drawn.

```sh
cargo run --release --example launcher -- --environment browser --example game
```

## Hosted demo and GitHub Pages

The root README links to the browser client-movement demo on GitHub Pages with
five clients and 1,000 ms average input latency with ±1,000 ms jitter (0–2,000 ms per input). The recording
links to that interactive configuration, while the recording itself uses the
[scripted choreography](../../../guides/usage/README.md#reproduce-the-reconciliation-recording). All processing stays in the visitor's browser.

The [Browser demo workflow](../../../.github/workflows/pages.yml) builds the
workload with Rust 1.90 and the isolated Bevy renderer with Rust 1.95 and installs the wasm-bindgen CLI version
specified by this example's manifest. It packages only `index.html`, the UI files,
and generated browser bindings/WebAssembly. Rust sources, local notes and the
rest of the repository are excluded from the deployed site.

Pull requests build and upload a Pages artifact without deploying. Relevant
pushes to `main`, or a manual workflow run on `main`, build and deploy it.
Manual runs on other branches only build the artifact. To enable hosting when
ready, select **Settings → Pages → Build and deployment → Source: GitHub Actions**
in the repository, then run the workflow from `main`. No deployment token secret
is needed; the deployment job uses GitHub's scoped token and OIDC permissions.

The public address is `https://humacss.github.io/contime/`; the README's startup
parameters select Client movement. Without parameters, the page opens the
existing Chain hash case. UI and WebAssembly imports use relative paths so they work
under the repository's `/contime/` project-site prefix.

### Movement presentation

A shared network timeline above the arena compares action time, scheduled due
time and actual arrival time on the same clock. Clients have color-coded rows on one time ruler; horizontal delay lines connect
actions to vertical delivery ticks. A double delivery tick highlights an older input arriving after a
newer input for that client.
The preview shows the next four waiting inputs plus arrivals retained to the
left edge of the visible clock window, up to 128
inputs; the waiting count includes the full simulated network queue.

The shared Bevy markers ease toward the latest queried positions, without
extrapolating beyond them or feeding visual state back into `contime`. New markers
fade in; removed markers fade out. Solid client-colored paths cover replaced
paths at exact overlaps. Replaced paths fade as dim dashes in the same client
color; faint gray dashes mark the intended routes in both arenas. The scripted
recording also shows the player’s intended route through the center.
Recent paths are brighter. Lighter arrows in each client’s color show delivered directions and squares show Stop inputs. Recent
inputs are larger and brighter; a brief size boost marks arrival at the historical
input location. Reduced-motion mode suppresses the size animation. Trails
remain queried history, rather than interpolated or predicted trajectories.
The short visual transitions add a little display lag; query results, keyboard
input delivery, monitoring and independent validation are unchanged. Reduced-motion
preferences disable these transitions. The movement page has no separate controls
or diagnostics panel; reload to restart it. The shared renderer still supports
pause and reset through its adapter.

The movement case uses a fixed 96 × 56 world-unit arena. Its model stops positions
at the walls, so queries and late-input replay observe the same collisions on all
hosts. The browser fits the whole arena into the canvas, rather than following
the player and letting other clients move off screen.
