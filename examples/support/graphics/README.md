# Shared Bevy presentation

One Bevy 0.19 renderer displays movement, service uptime, account balances and robotics for Native, WASI, threaded
WASI and Browser. This is an example-only workspace using Rust 1.95; it does not add
Bevy or a newer Rust requirement to ConTime's published crates.

## Choose a visualization

Choose **Visualizer** in the terminal launcher. The Bevy window offers **Client
movement**, **Service uptime**, **Account balances** and **Robotics / delayed perception**, using the same chooser on desktop and in the
browser. Click a card, press 1, 2, 3 or 4, or use the arrow keys and Enter. Native and
WASI hosts prepare the selected environment but start only the chosen workload
after selection; browser selection similarly creates no simulation beforehand.
Each case keeps its own configuration defaults and independent simulation.
Direct case commands and URLs still bypass this menu.

The shared catalogue lives in `../visualizer_catalog.rs`; the chooser lives in
`src/chooser.rs`. It communicates only a selected identity. Hosts create and run
the corresponding workload, so the presentation does not own execution policy.
New visualizations can register a choice and provide their own host configuration
and report renderer without borrowing movement logic.

## Navigation and explanations

Each active visualization has a top row of controls, followed by its name and explanation.
Service uptime explains its configured per-service event cadence, the gray
one-second grid, and why arrival-order processing can produce incorrect outage
history and totals. The description contrasts that with ConTime reconciliation.
**Reset** restarts the same case with its original settings and random seed.
**Back to visualizations** returns to the chooser; the next case starts a fresh
simulation. Menu cards explain what to watch before starting. The header uses
screen-sized text, with the diagrams fitted into the remaining area below it.

Navigation works while paused and after a finite run finishes. Native and WASI
hosts request stop and wait for final validation and successful child-process
exit before replacing the workload. Browser hosts drain within the configured
frame budget; their delivery clock continues while draining delayed input, but
hidden tabs remain suspended. Reset clears old rendering state and restarts time
at zero. Settings stay fixed; these controls do not edit configuration.
Direct desktop launches provide Reset; Back is available when launched with the
Visualizer catalogue. Direct browser case links also support returning to the chooser.
Finite graphical runs keep their final view open for Reset or Back; close the
window to exit. Counter/hash terminal behavior is unchanged.

## Service uptime

The [uptime case](../../cases/service_uptime/README.md) supplies real queried service
histories in paired rows: ConTime above, arrival-order state below. Both strips
share the same horizontal time axis. The default five service identities are soft cyan (#63CEDB), soft magenta (#E580BF), soft violet (#A28AE5),
light cocoa (#BE9478) and ivory (#D5D4C7),
reserving green, amber and red for healthy, degraded and down status.
The right-hand summaries show whole-run seconds for ✓ healthy uptime, × down,
and ! degraded. These three displayed totals are disjoint: degraded time is
subtracted from the backend’s inclusive uptime for the ✓ value. The visible
history remains bounded. Long runs use compact seconds notation.

The input timeline uses the live delivery clock, independently of the state panels'
last completed query time. This matters when conservative retention temporarily
holds the query horizon behind newly delivered reports. Each service has its own
row and identity color; double delivery ticks mark out-of-order reports. Old marks
are clipped rather than collecting at the axis boundary. The fixed ten-second
window places Delivery at 65%, leaving room for pending long-delay reports.
Timeline symbols (healthy check mark, degraded exclamation mark, down X) match their service-colored
delay lines, including opacity. Delivery animations retain that identity color.
Health strips and their permanent transition symbols retain the status colors.
Larger glyphs have a near-black outline so they remain readable over matching
health strips; the history markers extend beyond the strips without blank badges.
The workload defaults to a 250 ms minimum delivery delay after jitter.
Pending lines retain enough contrast to stay visible against the
background. Overlap tracks stay fixed as inputs enter or leave the preview;
symbol clearance is included even for zero-delay reports.
A delivered symbol travels to both strips together: original time above, arrival
time below. These bounded, pause-aware animations wait for the queried history
to include the arrival. The corresponding history report, its downtime totals,
and its permanent transition symbols become visible together when the symbols
land, 450 ms later. The network timeline stays live. The history viewport has a fixed ten-second
scale from startup and interpolates buffered query timestamps on every render
frame. Its right edge holds the last visible status between reports, without
changing queried totals or revealing a new status before landing. If no newer
query is buffered, it holds position rather than inventing a query horizon.
Pause freezes scrolling and the handoff;
reset clears it, and reduced motion or final validation reveals the latest report
immediately. Animation never delays workload processing.

`src/uptime.rs` owns this presentation. Both cases reuse `src/timeline.rs` delivery
marks, the label pool, transport, canvas, pause/reset state and window lifetime. Uptime does not borrow movement geometry or invent client paths.

Native transport retains the `contime-workers-v2` prefix and accepts exactly one
`movement`, `uptime`, `accounts` or `robotics` payload. Browser hosts call `update_scene`,
`update_uptime`, `update_accounts` or `update_robotics` on the same graphics module. Available time includes degraded time.
Finished reports display exact final state and a Finished label.

The default source cadence is exactly one second for every service. Delivery jitter
changes when those reports arrive, never their action timestamps. Both the input
timeline and the paired history display have an absolute one-second grid; history
grid lines span all service rows. Thus corrected transitions align to the grid,
while arrival-order transitions can fall between columns or in the wrong order.
A custom health-change cadence remains available through the launcher.

## Reconciliation comparison

One network timeline feeds two equal-scale arenas. Faint dashed neutral-gray outlines
show each bot's intended square in both arenas. During the scripted recording,
the player also has a cross-shaped guide through the center. Free keyboard
movement has no prescribed route and therefore no player guide. These static
guides come from the example's own input patterns and motion rules; they never
replace queried paths or affect processing.

**With ConTime** on the left
shows actual queried state: late turns can temporarily overshoot, then repair the
historical path. Dashed segments in the same client color retain those revisions while fading away. **Without reconciliation**
on the right changes direction at actual delivery time and keeps earlier movement
unchanged. Both receive exactly the same delivered inputs, start in the same places,
and share colors, speed, walls and display smoothing. The right-hand state belongs
to the example workload, not the renderer, and never affects ConTime or validation.

The four bots start in separate quadrants and repeat square routes. Interactive
turns share the same source ticks (once per second by default), with distinct
movement phases and independent delivery delays. The recorded choreography
retains its scripted timing. Their alternating loop alignments leave space
between routes.
Their corrected square loops remain closed after late inputs arrive; arrival-time
loops deform. The keyboard-controlled client still appears in both views. Turn rate
and speed determine side length, so extreme settings can reach the shared walls.
The README GIF shows ten seconds of both views with scripted delivery delays.
The player moves out from the center and back along each axis; the four bot loops
stay separate. The recording ends when the final Stop is visible in both views,
with a brief hold on the centered player and the unreconciled offset. See the [recording instructions](../../../guides/usage/README.md#reproduce-the-reconciliation-recording).

The renderer knows positions, paths, corrections and arena bounds. It does not generate
events, process ConTime work or decide whether state is correct. The selected workload
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

Above the arena, one shared time ruler shows up to 128 inputs: up to 64 pending
deliveries, with recent delivered history filling the remaining slots. Each action carries a
direction icon at its original time; colored row markers identify clients without
letters or names. On first appearance, the direction icon travels from its client’s row
marker to its action time, then its delay line grows to the vertical delivery tick. This
takes at most 400 ms and shortens to fit the remaining network delay. Already
arrived or overdue inputs appear fully placed, so entrance animation never
continues after delivery. It freezes on Pause, clears on Reset, and is suppressed
with reduced motion. Horizontal delay lines connect those icons to
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

Completed query updates use a bounded presentation buffer of 450 ms. Two
synchronized tracers carry the same client-colored direction icon from the shared
timeline delivery point into both arenas. The left tracer lands at the input's
historical position; the right lands where that input actually arrived. Both
arenas reveal the matching query update when those tracers land, so the turn and
path correction do not precede the arrow. This adds visual latency only: input
delivery, ConTime processing, queries and validation continue normally.

Each flight keeps the destination of its own queued query; later corrections do
not move it toward an arena update that is still waiting. Timeline entrances
finish by network delivery, before the arena handoff starts. The
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
camera's world scale and the line width used for paths. Headings use 18–24 px,
captions 14–16 px, legends 12–14 px and brief arrival notices 12 px. Compact titles below 900 px
and concise captions below 640 px keep both arenas side by side. Supplementary
legends and reordered words hide below 640 px; double delivery ticks remain. UI nodes provide
wrapping and reusable placement anchored to the arena/timeline geometry; no HTML
labels or network font requests are needed. The Inter font and its SIL Open Font
License are in `assets/fonts/`.

## Run

From the repository root:

```sh
cargo run --release --example launcher
```

Choose Visualizer and any environment, then select a case in the graphical window. Native and both WASI choices open a
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

For movement, WASD/arrows choose a persistent direction; Space stops. Inputs pass through the
workload's simulated delay. Closing a desktop window, Escape or Q requests a
stop, then waits for the workload to drain and validate. A successful exit needs
both the final validated report and a successful workload process exit. Errors
retain a bounded diagnostic tail. The presenter reaps its child on cleanup.

Browser movement and uptime start automatically, with startup settings fixed for the run.
Press P to pause or resume; a Paused label appears in the visualizer. Hiding the tab
automatically pauses its simulation clock, work and animations. Returning resumes
only an automatically paused run; a manual pause stays paused. Pending deliveries
and operation deadlines retain their remaining simulation time.
Reload to restart, or close the page to stop. Counter and Chain hash retain their
browser controls and diagnostics. Reduced-motion preferences disable marker transitions.

## Code

- `src/navigation.rs`: shared top bar, case explanations and host navigation requests.
- `src/viewport.rs`: reserve space for navigation without covering graphs.
- `src/uptime.rs`: service health strips, status symbols and shared timeline placement.
- `src/uptime_playback.rs`: queried-history reveal synchronized with delivery symbols.
- `src/uptime_transfers.rs`: paired delivery-to-history animations.
- `src/uptime_tracks.rs`: stable service-row tracks with symbol clearance.
- `src/timeline.rs`: shared time placement and delivery marks.
- `src/comparison.rs`: equal-scale left/right movement report placement.
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

## Account balances

The third visualization compares timestamp-reconciled provisional payments with
arrival-order decisions. Both sides deduplicate transaction IDs. Its independent
workload owns the scripted deliveries, snapshots and validation; `src/accounts.rs`
only draws the delivery timeline, paired ledgers, balance histories and delivery
handoffs. See [the case README](../../cases/account_balances/README.md) for the
scenario and its limits. These are provisional account decisions, not settlement.

## Robotics

`src/robotics.rs` independently presents a known warehouse, simulated physical
truth, pose estimates, historical trails and proposed routes. Its camera timeline
shows capture time and result availability. The left estimate replays from capture
time; the right applies the observation at delivery time. Both receive all results.
State revisions are buffered for the short delivery handoff, with bounded fading
paths and routes. Reports carry revisions so a changed query at the same timestamp
still updates the view. Pause freezes presentation time; Reset clears every cache.
See [the robotics case](../../cases/robotics/README.md) for application rules and
limitations. The presentation does not run estimation or control physical motion.
