# Run and read the examples

Start with a small walkthrough, then explore the same idea under a configurable
live load. Native examples use the root package; run these commands from the
repository root. Use `--release` when comparing performance.

The graphical launcher and WebAssembly hosts require a [repository checkout](https://github.com/humacss/contime):
Cargo does not include their standalone manifests in the published crate archive.

| Example | Command | Read first |
| --- | --- | --- |
| README ordered list | `cargo run --example readme --features macros` | [`readme.rs`](readme.rs), simple IDs and intermediate late-insertion assertions |
| Composite Content IDs | `cargo run --example composite_ids --features macros` | [`composite_ids.rs`](composite_ids.rs), field selection and mapping |
| Event registration | `cargo run --example event_registration --features macros` | [`event_registration.rs`](event_registration.rs), defaults and overrides |
| Module-local event subsets | `cargo run --example module_subsets --features macros` | [`module_subsets.rs`](module_subsets.rs), nested composition and snapshot-specific filtering |
| Explicit traits | `cargo run --example explicit_traits` | [`explicit_traits.rs`](explicit_traits.rs), the same ordered list without `contime` macros |
| Dependent modules | `cargo run --example modules --features macros` | [`modules.rs`](modules.rs), shared dependencies and multiple snapshot types |
| Order-sensitive hash chains | `cargo run --release --all-features --example chain_hash` | [`cases/chain_hash/model.rs`](cases/chain_hash/model.rs), then [`cases/chain_hash/setup.rs`](cases/chain_hash/setup.rs) |
| Client movement | `cargo run --release --all-features --example client_movement` | [`cases/client_movement/model.rs`](cases/client_movement/model.rs), then [`cases/client_movement/setup.rs`](cases/client_movement/setup.rs) |
| WebAssembly without a browser | See [`environments/wasi/README.md`](environments/wasi/README.md) | [`support/composition/wasi/main.rs`](support/composition/wasi/main.rs), then [`support/load/verification.rs`](support/load/verification.rs) |
| WebAssembly in a browser | See [`environments/browser/README.md`](environments/browser/README.md) | Shared WebAssembly Rust code, then [`environments/browser/src/clock.rs`](environments/browser/src/clock.rs) |

Start the launcher with `cargo run --example launcher`; it handles build features
for you. Native workload targets compile the module derive, Runtime, Manual, monitoring, and simulation
together. If running one directly, use `--all-features`, then choose
`--execution runtime|manual` for live cases. Ordinary library consumers still get
only Runtime by default.

The `readme` ordered-list walkthrough stays intentionally small and uses Runtime. It is run
directly with `cargo run --example readme --features macros` and is not offered by the launcher. The separate
[Manual code sample](cases/counter/manual.rs) is included in the Rust documentation;
it is not another Cargo example target. The live counter workload remains an
internal test and benchmark fixture.

## Folder layout

Small, self-contained walkthroughs live directly under `examples/`. Each one
includes its own definitions, startup, assertions and shutdown, and runs without
the launcher or shared UI. Read them in the order that matches what you need:

1. [`readme.rs`](readme.rs): field IDs, timestamps, late insertion and retries.
2. [`event_registration.rs`](event_registration.rs): default event registration
   and a singular override for renamed identity fields.
3. [`composite_ids.rs`](composite_ids.rs): select multiple identity fields and
   map them with Content ID; use the same mapping for queries.
4. [`modules.rs`](modules.rs): combine snapshot types and nested modules while
   deduplicating a shared dependency.
5. [`module_subsets.rs`](module_subsets.rs): restrict one snapshot's accepted
   events while another snapshot keeps its full event set.
6. [`explicit_traits.rs`](explicit_traits.rs): implement the small README model
   without any `contime` macros, using the lower-level execution contracts.

These finite demonstrations use idle waits only while submissions are paused so
that their assertions are easy to follow. They are not live synchronization or
throughput examples. Configuration and shutdown code is repeated intentionally:
each file can be read and copied without following support modules.

The folders below organize the live diagnostic examples:

- [`cases/`](cases/README.md): application models, lane setup, simulator callbacks,
  correctness checks.
- [`environments/`](environments/README.md): generic `contime` startup, native
  process lifetime, WASI hosting and browser clock/frame scheduling.
- [`support/`](support/README.md): case selection and composition, shared
  configuration, monitoring bridges, load control, validation, UI and launcher.

Monitor and Simulator remain library crates under `crates/`; these examples
compose their public APIs. Manual versus Runtime is an execution choice within
an environment, rather than a separate application case. The launcher composes the same live cases as the direct commands.

## Start from one place

```sh
cargo run --release --example launcher
```

The [launcher](support/launcher/README.md) selects the environment, execution, example,
and applicable settings, then starts the existing example. It can build and run
the standalone WASI command or serve and open the browser host. Browser settings
stay fixed for that launched run; Ctrl-C in the launcher stops its local server.
Individual commands above remain available.

## Compose a definition, execution model, and host

The movement and hash folders define application behavior: events,
snapshots, destination IDs, and how an event changes a snapshot. Start with a
folder's `model.rs`. Its small `setup.rs` adapts that definition to the shared
example drivers; it contains no worker loop, pacing, backpressure, or UI.
Live entry points under
`support/entrypoints/` select the native composition. Portable compositions and
case selection live in `support/composition/`; environments supply generic host
capabilities and execution resources.

Execution is a separate choice: **Runtime** manages background threads, while
**Manual** makes one `process()` call per host-loop turn. Native live entry
points accept `--execution runtime|manual`. Runtime is the default. To use Manual:

```sh
cargo run --release --all-features --example client_movement -- --execution manual
cargo run --release --all-features --example chain_hash -- --execution manual
```

Environment is a third choice. The WASI command and browser host import the same
hash and client movement definitions and their shared live drivers; select
`chain-hash` or `client-movement` in any host. They do not depend on each other.
Standard WASI and Browser use Manual. Standalone threaded WASI also verifies
the existing Runtime, using Wasmtime 36.x LTS and the `wasm32-wasip1-threads`
target. Choose WebAssembly / Node (WASI) or WebAssembly / Wasmtime threads in the
launcher; execution modes follow the chosen environment.

| Host | Definitions exposed | Execution |
| --- | --- | --- |
| Native live commands | Movement, hash | Runtime or Manual |
| Standalone WASI / Node | Movement, hash | Manual |
| Standalone WASI threads / Wasmtime 36.x LTS | Movement, hash | Runtime or Manual |
| Browser | Movement, hash | Manual with a frame budget |

Shared [`support/`](support/README.md) contains settings, execution adapters,
production control, validation and UI. All live cases use simulator iterators
and worker-monitor feedback. WASI keeps computation and full validation in
WebAssembly, sending worker readings and a bounded snapshot preview to the
shared Bevy presenter. Browser hosts use the same controller and Manual
backend through a frame-budgeted adapter. Hash definitions and delivery
rules are shared across environments.

The single-file walkthroughs above teach the public API independently of this
live diagnostic harness.

The launcher calls the hash-chain example **Benchmark** and the movement example **Game** and starts it with fixed defaults
in any environment. Native and threaded WASI use Runtime; standard WASI and the
browser use Manual. Its fixed input cadence
and historical query cycles differ from the throughput cases described below;
see its [guide](cases/client_movement/README.md).

## Choose the load before starting

The launcher lists the example first and the environment second. Game starts with
fixed defaults. Benchmark shows execution, workers, snapshots, event rate, out-of-order delivery, query
frequency and retention first. The Advanced section is also visible by default and contains
checkpoint spacing, fanout, skew, backlog and browser processing limits. Arrow keys select common
values; typing replaces a value. Enter validates and starts; Esc cancels. Settings
stay fixed during a run and can also be supplied on the command line.

| Setting | Default | Meaning |
| --- | --- | --- |
| Workers | Available logical CPUs for Runtime; one for Manual | Independent worker instances |
| Snapshots | 1,000 per worker | Population of possible destination IDs, up to 1,000,000 |
| Event rate | Maximum | Adaptive load probe, or a numeric source-events/s cap |
| Out-of-order delivery | 100% | Fraction of each group reversed, from ordered to fully reversed |
| Query frequency | 20/s | Upper bound for hash/movement checks at completed windows |
| History retention | 10 seconds | Retained application-time interval |
| Checkpoint spacing | 128 events | Approximate interval between saved states |
| Average/maximum snapshots per event | 1.0 / 2, capped by population | Mean and cap for distinct destinations |
| Snapshot skew | 0 | Traffic concentration, 0–100 |
| Pending source events | Numeric rates only | Optional cap on submitted events not yet monitor-confirmed |

Maximum starts probing at 100,000 source events/s. It confirms bounded windows
from exact insertion totals, accepted processing targets and no pending replay
on every worker. Fast windows probe upward; slow ones reduce offered load.
Maximum has no user-configured pending limit. Numeric rates pace events and
retain a configurable source window. One event targeting several snapshots still
uses one source slot, so a one-event window can contain a multi-target event.

Worker/snapshot-dependent defaults follow worker changes until edited. Use
`--no-setup` to run directly, `--headless --duration 5` for a finite comparison,
and `--execution manual` for Manual. Enable `manual` to make both APIs available.
Native chain hash headless runs default to five seconds. Native movement and
browser/WASI runs stop automatically only with an explicit duration. Explicit
durations have no upper cap; omit the duration for an interactive run that
continues until stopped. Press q, Escape or Ctrl-C to stop native runs.

## Fanout and distribution

The average controls the expected number of distinct destinations per event;
the maximum caps it. An average of 1.1 with maximum 2 produces about one shared
event in ten, rather than promising an exact ratio in every finite sample.
Traffic skew concentrates destinations as it approaches 100; it does not promise
coverage or a fixed proportion of active snapshots. Hash groups retain a common
primary destination so reordered delivery exercises ordering within a history.

```sh
cargo run --release --all-features --example chain_hash -- \
  --snapshots 1000 --workers 4 --average-snapshots-per-event 1.5 \
  --max-snapshots-per-event 3 --snapshot-skew 90 --headless --duration 5
```

## Understand the measurements

The headline target is offered source events/s. Average completed events/s is
monitor-confirmed source events divided by elapsed run time. Outstanding means
submitted source events not yet confirmed as a complete window, so it includes
finished work awaiting the next reading. Worker totals and rates count target
applications; one source event targeting three snapshots contributes three
insertions. Raw replay can include repeated traversals and never measures
unique completed source events.

Queries inspect and validate state; they do not confirm processing or grant
production credit. Hash checks freeze new generation/advancement while their
captured prefix is queried. Movement instead keeps generating delayed inputs but
pauses their submission and `contime` advancement during its historical query cycle. Query work still consumes time, and
browser frame cadence, generation and UI remain part of measured throughput.
Use release builds, unchanged settings and headless comparisons when profiling.
Prepared-input benchmarks under `benches/` measure a different workload.

The graphical Chain hash view shows a recent throughput graph, worker activity,
and verification status across the complete snapshot population.

## Validation and shutdown

Native hosts stop production, confirm outstanding work through monitoring,
validate all final state, restore the terminal, print a summary and exit without
tearing down retained histories. Reusable runners and tests shut down normally.
WASI guests likewise flush a final validated record before process exit. Browser
sessions preserve their frame allowance and pause/resume ownership. The independent finite
portable scenario covers late events, duplicates, rejection and pruning.
