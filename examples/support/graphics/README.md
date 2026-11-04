# Shared Bevy presentation

One Bevy 0.19 renderer displays the movement scene and hash dashboard
for Native, WASI, threaded WASI and Browser. This is an example-only workspace using Rust 1.95; it does not add
Bevy or a newer Rust requirement to `contime`'s published crates.

## Reconciliation comparison

One network timeline feeds two equal-scale arenas. Faint dashed neutral-gray outlines
show each bot's intended square in both arenas. During the scripted recording,
the player also has a cross-shaped guide through the center. Free keyboard
movement has no prescribed route and therefore no player guide. These static
guides come from the example's own input patterns and motion rules; they never
replace queried paths or affect processing.

**With `contime`** on the left
shows actual queried state: late turns can temporarily overshoot, then repair the
historical path. Dashed segments in the same client color retain those revisions while fading away. **Without reconciliation**
on the right changes direction at actual delivery time and keeps earlier movement
unchanged. Both receive exactly the same delivered inputs, start in the same places,
and share colors, speed, walls and display smoothing. The right-hand state belongs
to the example workload, not the renderer, and never affects `contime` or validation.

The four bots start in separate quadrants and repeat square routes with synchronized
one-second turns and client-specific phases. Their alternating loop alignments leave space
between routes.
Their corrected square loops remain closed after late inputs arrive; arrival-time
loops deform. The keyboard-controlled client still appears in both views. Turn rate
and speed determine side length, so extreme settings can reach the shared walls.
The README GIF shows ten seconds of both views with scripted delivery delays.
The player moves out from the center and back along each axis; the four bot loops
stay separate. The recording ends when the final Stop is visible in both views,
with a brief hold on the centered player and the unreconciled offset. See the [recording instructions](../../../guides/usage/README.md#reproduce-the-reconciliation-recording).

The renderer knows positions, paths, corrections and arena bounds. It does not generate
events, process `contime` work or decide whether state is correct. The selected workload
owns the simulator, worker monitor and independent validation. Markers ease toward
queried positions without predicting ahead. The icon under each character shows the
queried heading (an arrow), or a dot for Stop. It does not infer direction from
positional smoothing or from a newly arrived input. Connected client-colored paths show
the latest queried trajectory; displaced dashed segments in the same client color fade out only where comparable
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
occurs later. The display shows the simulated network, not `contime`'s internal worker
queue.

Completed query updates use a bounded presentation buffer of 450 ms. Two
synchronized tracers carry the same client-colored direction icon from the shared
timeline delivery point into both arenas. The left tracer lands at the input's
historical position; the right lands where that input actually arrived. Both
arenas reveal the matching query update when those tracers land, so the turn and
path correction do not precede the arrow. This adds visual latency only: input
delivery, `contime` processing, queries and validation continue normally.

Each flight keeps the destination of its own queued query; later corrections do
not move it toward an arena update that is still waiting. Very short deliveries
finish any remaining timeline entrance before the arena handoff starts. The
buffer and tracers freeze on Pause and clear on Reset. Reduced motion and final
completion bypass the buffer and show exact state immediately. Old inputs outside
the bounded preview do not play stale delivery animations.

The game view focuses on the input timeline, arena and explanatory legends.
Timeline arrows show direction and dots mean Stop; ticks mark delivery time,
with double ticks for out-of-order arrivals. Arena arrows/dots show current
queried direction, while lighter arrows/squares annotate delivered movement/Stop
inputs at historical positions. Browser shows only this shared graphical view;
Native and WASI retain their validation summary in the launching terminal. Monitoring, query validation and
simulation continue unchanged.

Gold identifies the player; mint, sky blue, coral and lilac identify the bots. A pale
ring distinguishes the player in both the arena and timeline. Input icons use a lighter
shade of the same hue, while current paths use a darker shade. Replaced paths use the client’s hue with reduced opacity and dashes; only the faint intended guides are gray.

## Text

All labels use Bevy UI text with an embedded Inter font, shared by desktop and
browser builds. Font sizes are in logical screen pixels, independently of the
camera's world scale and the line width used for paths. Headings use 15–20 px,
captions 14–16 px, legends 12–14 px and brief arrival notices 12 px. Arena titles
stay side by side on smaller screens. Reordered notices appear next to the relevant
delivery tick and fade while the double tick remains. UI nodes provide wrapping and
reusable placement anchored to the arena and timeline geometry; no HTML labels or
network font requests are needed. The Inter font and its SIL Open Font
License are in `assets/fonts/`.

## Run

From the repository root:

```sh
cargo run --release --example launcher
```

Choose any live case and environment. Native and both WASI choices open a
Bevy desktop window. Browser compiles the same presentation to WebAssembly and
embeds it in the page. Runtime/Manual selection still chooses how the workload
runs. The benchmark compares confirmed throughput in green with the submission
target in blue. The same graph shows outstanding events in amber and workload
memory in violet. Event rates and the outstanding-event count share the left
scale, while memory uses the right scale. Hover to inspect a
sampled time. Click a reading to keep it selected as the chart moves, or click
outside the chart to release it. A pinned reading stays visible after it leaves
the chart. Each scale follows its visible values.
The chart expands to use the available width and unused worker-row space. The table
shows worker activity and verified hashes as counts and percentages, both overall
and per worker. A verification mismatch stops the workload and stays visible. The
memory figure is the workload process's current resident memory,
not the renderer or the whole machine. It is available on macOS and Linux native
hosts; browser and unsupported hosts show it as unavailable. Use the arrow keys
or Page Up and Page Down to browse workers.

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

Browser cases start automatically, with startup settings fixed for the run.
Press P to pause or resume; a Paused label appears in the game. Hiding the tab
automatically pauses its simulation clock, work and animations. Returning resumes
only an automatically paused run; a manual pause stays paused. Pending deliveries
and operation deadlines retain their remaining simulation time.
Use Reset to restart, or close the page to stop. All cases use this same
graphical interface in the browser. Reduced-motion preferences disable marker transitions.

## Code

- `src/comparison.rs`: equal-scale left/right report placement.
- `src/labels.rs`: regular screen-sized text with embedded Inter and a bounded UI pool.
- `src/palette.rs`: shared client identity colors and lighter input shades.
- `src/scene.rs`: shared Bevy camera, walls, paths and markers.
- `src/tracks.rs`: stable overlap layout within each client row.
- `src/playback.rs`: bounded query presentation timed to arrow landings.
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
