# Bevy movement presentation

One Bevy 0.19 renderer displays the queried movement scene for Native, WASI, threaded
WASI and Browser. This is an example-only workspace using Rust 1.95; it does not add
Bevy or a newer Rust requirement to ConTime's published crates.

The renderer knows positions, paths, corrections and arena bounds. It does not generate
events, process ConTime work or decide whether state is correct. The selected workload
owns the simulator, worker monitor and independent validation. Markers ease toward
queried positions without predicting ahead. The icon under each character shows the
queried heading (an arrow), or a dot for Stop. It does not infer direction from
positional smoothing or from a newly arrived input. Connected client-colored paths show
the latest queried trajectory; displaced gray segments fade out only where comparable
historical samples disagree. Current paths cover revisions at exact overlaps. Both the
queried trail and revision fade default to five seconds. Duplicate revised segments are
drawn once. Older segments are dimmer. Lighter arrows in each client’s color mark
delivered movement inputs at their historical positions; squares mark Stop inputs. A
brief size boost marks arrival, independently of the input timestamp or query cadence.
Input markers use the delivered-input reference timeline, so they can appear before the
next queried path and move when an earlier input arrives. Markers remain within the
trail window (up to 128 recent inputs); reduced-motion mode suppresses the size
animation. Recent inputs are larger and brighter than older inputs. A fixed camera fits
the whole arena, including when the player reaches a wall.

Above the arena, one shared time ruler shows up to 128 inputs: the next four scheduled
deliveries and delivered history within the visible window. Each action carries a
direction icon at its original time; colored row markers identify clients without
letters or names. On first appearance, the direction icon travels from its client’s row
marker to its action time, then its delay line grows to the vertical delivery tick. This
takes 400 ms, separately from network delivery; it freezes on Pause, clears on Reset,
and is suppressed with reduced motion. Horizontal delay lines connect those icons to
vertical delivery ticks. Each client has a dedicated, color-coded row, including when it
has no visible inputs. Overlapping ranges use stable tracks inside that row; disjoint
ranges reuse tracks. If actual delivery extends a delay line into another input,
the conflicting track is reassigned; otherwise existing tracks stay in place. Icons and delivery animations follow their assigned track; the
player is first, followed by the other clients. The timeline spans six seconds at normal
delays, expanding for longer latency settings. Unlabeled vertical divisions mark whole
seconds. The timeline has no numeric time labels; actions move left past a fixed
Delivery line. Dimmer delay lines show waiting inputs; brighter lines show arrivals; a
double delivery tick marks an older action arriving after a newer action from the same
client. A brief “reordered” label explains the mark, then fades while the double tick
remains. A quiet dot on the delay line preserves scheduled due time when actual handoff
occurs later. The display shows the simulated network, not ConTime's internal worker
queue.

On delivery, a short tracer carries the client-colored direction icon from the
timeline's delivery point to the input's historical position in the arena. This is a
presentation effect; query results continue updating normally. A delivery that happens
during entrance waits for that visual entrance to finish before launching its tracer;
simulation and queries do not wait. Tracers are bounded, freeze on Pause, clear on
Reset, and are disabled with reduced motion. Inputs outside the bounded historical
preview are not animated.

The game view focuses on the input timeline, arena and explanatory legends.
Timeline arrows show direction and dots mean Stop; ticks mark delivery time,
with double ticks for out-of-order arrivals. Arena arrows/dots show current
queried direction, while lighter arrows/squares annotate delivered movement/Stop
inputs at historical positions. Browser shows only this shared graphical view;
Native and WASI retain their validation summary in the launching terminal. Monitoring, query validation and
simulation continue unchanged.

Gold identifies the player; mint, sky blue, coral and lilac identify the bots. A pale
ring distinguishes the player in both the arena and timeline. Input icons use a lighter
shade of the same hue, while current paths use a darker shade. Revised paths stay gray.

## Run

From the repository root:

```sh
cargo run --release --example launcher
```

Choose Client movement and any environment. Native and both WASI choices open a
Bevy desktop window. Browser compiles the same presentation to WebAssembly and
embeds it in the page. Runtime/Manual selection still chooses how the workload
runs. Counter and Chain hash retain their terminal dashboards.

Install Rust 1.95 for presentation and its browser target when needed:

```sh
rustup toolchain install 1.95.0 --profile minimal --target wasm32-unknown-unknown
```

On Ubuntu, native builds and renderer tests also need the X11 and Wayland
build dependencies for the enabled window backends:

```sh
sudo apt-get install pkg-config libx11-dev libwayland-dev libxkbcommon-dev
```

The launcher reports missing tools; it does not install them. Browser bindings
also need the wasm-bindgen CLI matching the example manifests.

WASD/arrows choose a persistent direction; Space stops. Inputs pass through the
workload's simulated delay. Closing a desktop window, Escape or Q requests a
stop, then waits for the workload to drain and validate. A successful exit needs
both the final validated report and a successful workload process exit. Errors
retain a bounded diagnostic tail. The presenter reaps its child on cleanup.

Browser movement starts automatically, with startup settings fixed for the run.
Reload to restart, or close the page to stop. Counter and Chain hash retain their
browser controls and diagnostics. Reduced-motion preferences disable marker transitions.

## Code

- `src/palette.rs`: shared client identity colors and lighter input shades.
- `src/scene.rs`: shared Bevy camera, walls, paths and markers.
- `src/tracks.rs`: stable overlap layout within each client row.
- `src/transfer.rs`: bounded delivery-to-arena tracers.
- `src/network.rs`: shared action/delivery timeline and arrival highlights.
- `src/paths.rs`: connected paths, directional input markers and arrival pulses.
- `src/entrance.rs`: client-to-timeline entrance and delay-line growth.
- `src/animation.rs`: presentation-only smoothing and fade state.
- `src/bridge.rs`: latest-value report slot and pause/reset state.
- `src/web.rs`: browser exports; JavaScript retains the workload frame budget.
- `src/native.rs`: bounded report/control pipes to the selected workload process.
- `src/main.rs`: desktop window and keyboard/close handling.

Native uses the existing portable workload compiled to a native executable.
WASI uses that workload compiled to the selected WASI target. The frontend never
substitutes native simulation for a selected WebAssembly workload. Both use the
existing versioned worker-report protocol; at most two reports are buffered.
Browser workload and renderer are separate WebAssembly modules, so rendering
stays outside the Manual processing budget, just as the previous canvas drawing did.
Rendering and other example overhead still affect observed performance.

## Verify

```sh
cargo +1.95.0 test --manifest-path examples/support/graphics/Cargo.toml
cargo +1.95.0 clippy --manifest-path examples/support/graphics/Cargo.toml --all-targets -- -D warnings
cargo +1.95.0 build --manifest-path examples/support/graphics/Cargo.toml --release --lib --target wasm32-unknown-unknown
```
