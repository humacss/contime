# Client movement

One event type changes the movement direction of one character snapshot. Client 1
is yours; the other clients are driven by a simulator iterator. Bot and keyboard direction changes keep
their original timestamps and pass through the same delayed delivery queue.
Initial stopped characters are seeded before simulated traffic starts.
The example includes simple arena-wall collisions; it has no network server or client-side prediction.

```sh
cargo run --release --all-features --example client_movement
cargo run --release --all-features --example client_movement -- --execution manual
```

The direct commands above retain the terminal view. For the shared Bevy graphics,
choose **Visualizer** in the central launcher with Native, WASI, WASI
Threads or Browser, then **Client movement** in the graphical window. To configure
all movement-specific settings before starting, use the launcher's
`--example client-movement` option to open this case directly. The browser uses Manual and its processing budget per frame;
WASI Threads also supports Runtime. The direct-case startup form shows
client count, workers, input/query frequencies, delivery latency/jitter and trail
and fade durations, with retention and checkpoint spacing in the ConTime group.
The launcher collects these settings before starting the example; direct launches
use the same schema and editor. Settings are fixed for the run. Use WASD or arrows to choose
a persistent direction, Space to stop, and q / Esc / Ctrl-C to quit. Keyboard
inputs experience the same latency and reordering as bot inputs. Terminal key
release events are not required. Client count is limited to one through five, including the human-controlled client.
Use Counter or Chain hash to explore larger workloads.

## Events, snapshots and the lane

Read `model.rs`, then `setup.rs`. A `MovementInput` names a client, timestamp,
event ID and direction. Its snapshot ID is the client ID. `Character` implements
`ApplyEvents<MovementInput>`: integrate the previous direction up to the event's
time, then select the new direction. This relationship is the lane; `setup.rs`
only connects it to example support and the generic execution adapters.

Positions use fixed-point integers. Movement speed defaults to six world units per second. A separate
integration timestamp preserves the position anchor when ConTime updates snapshot
metadata during queries or pruning. Rendering projects the accepted direction
to the query time; it never applies keyboard input directly to a displayed character.
The player starts at the center of the 96 × 56 world-unit arena. Four bots start
in separate quadrants: the spacing setting gives their horizontal offset and half
that value gives their vertical offset. Defaults are 16 horizontally and 8 vertically.
Their square phases alternate the side from which each loop starts, keeping the
routes apart. Extreme spacing is bounded inside the arena; zero spacing deliberately
starts everyone together. Positions stop at a wall until a new direction turns
them inward. Spacing and movement speed make delayed turns and revised paths easier
to see.

## Late inputs and rendering states

Defaults are five clients, one direction change per bot per second, 500 ms average
latency with ±500 ms jitter (0–1,000 ms per input), and up to 20 completed query updates per second.
Native, WASI and browser hosts share these defaults. At the default one-second
bot cadence, late turns still cause corrections, but the 0–1,000 ms delay range
does not normally reorder a bot's inputs. To demonstrate that too, use 1,000 ms
average latency with ±1,000 ms jitter, or increase the input frequency. Delivery is simulated in support; ConTime is unchanged.

Upcoming bot inputs are generated ahead within the timeline's visible future window.
Their action timestamps stay on the shared ticks; each delivery is scheduled from
that action time, not from preview generation. Only due inputs enter ConTime.
Stopping discards future actions and drains inputs whose action time has passed.
The scripted recording keeps its original schedule.

The whole arena stays in view, with walls around its boundary. Faint, static
dashed neutral-gray outlines mark the bots' intended squares in both comparison views.
Only the scripted recording adds a guide for the player, showing its intended
trips along the central axes. These guides illustrate the input pattern rather
than queried or predicted state. In the shared graphical view, gold with a ring identifies you; mint, sky blue,
coral, and lilac identify the bots. Direction icons, timeline rows, delivery
animations and current paths share each client’s color. Old paths fade
as dim, dashed traces in the client’s color only where the latest query disagrees for the same client at the same
historical timestamp; matching points and unmatched sliding-window endpoints are
omitted. Delivered input markers briefly grow on arrival; ordinary motion is not
counted as reconciliation. Inputs arriving together
may produce one visible correction.

Rendering refreshes independently while queries are pending. A query cycle samples
the retained trail every 200 ms plus its current endpoint and validates all clients,
including those omitted by the bounded visual preview. At most 320 old rendering states are kept, covering a five-second fade at up to 60 query updates per second.
Trails live in presentation, not character snapshots.

Worker-monitor readings confirm the submitted prefix before querying. Submissions
and advancement pause during that query cycle so late arrivals cannot invalidate
its captured reference; input generation, the simulated network, keyboard input
and fading continue. The direct terminal view includes worker totals and average
rates; graphical hosts focus on the arena, timeline and legends.
This real-time demonstration is not Maximum throughput mode; use Counter for that.

## Configuration and validation

```sh
cargo run --release --all-features --example client_movement -- \
  --no-setup --clients 5 --workers 4 --input-hz 5 --query-hz 20 \
  --average-latency-ms 350 --jitter-ms 300 --trail-ms 2000 --fade-ms 1000
cargo run --all-features --example client_movement -- \
  --headless --duration 2 --clients 5
```

Average latency can be 50 ms, one second, several seconds or longer. Use
`--average-latency-ms 50` or `--average-latency-ms 1000`; `--latency-ms` is an alias.
`--jitter-ms` is the maximum deviation on either side of the average. Its effective
radius is capped at the average: 50 ms average with 300 ms requested jitter gives
uniform delays from 0–100 ms and still averages 50 ms. Zero jitter gives a fixed
delay. Constant latency alone does not reorder inputs.

Retention starts at 10000 ms and automatically grows to cover maximum delivery
delay plus the trail and a 500 ms margin, including edits in the startup form.
An explicit `--retention-ms` disables automatic sizing and must cover that window.
The existing retention limit is one day; combinations exceeding it are rejected.
Shutdown allows the configured maximum latency before its additional drain timeout. Checkpoint spacing defaults to 128 and is
set with `--checkpoint-interval`. Runtime uses up to four available logical CPUs
by default; Manual uses one worker. Explicit durations have no upper cap. Omit
duration to run until stopped. Input/client limits bound accidental overload;
the simulated network refuses more than one million queued inputs.

Quitting stops new inputs, delivers the remaining queued inputs, validates final
positions and counts, restores the terminal and shuts down the backend. The
reference compacts old input prefixes as retention advances. Tests exercise actual
late turns after materialized queries, duplicate inputs, both execution modes,
retention, keyboard decoding, delayed delivery and fading/correction detection.

```sh
cargo test --all-features --example client_movement
```

## Shared driver, different hosts

`support/movement/driver.rs` owns delayed delivery, monitor-confirmed query cycles,
reference validation and fading rendering states. Each turn makes at most one
worker `process()` call. Hosts supply elapsed time and keyboard directions;
none of the browser code sleeps or starts background threads. Configuration and
case definitions are shared across environments.

The launcher uses the same Bevy presentation for Native, WASI and Browser.
Direct native example commands retain the terminal map. For WASI the presenter
forwards keys to the guest's input pipe and draws its bounded reports; all
simulation, queries and validation remain in WebAssembly. In the browser, click
the map and use WASD/arrows or Space. Old queried paths fade and revised paths
appear as faded dashes in the client’s color. Visual reports cover the camera region and cap characters/path
points to bound transport and drawing; every configured client remains validated.

## Spacing and speed

The launcher's **Client movement** group includes **Client spacing** and
**Movement speed** in every environment. Defaults are 16 world units between
starting positions and 6 world units/second. Both accept whole numbers from
0 to 1,000,000; zero spacing starts clients together and zero speed keeps them
stationary while inputs and replay validation continue.

```sh
cargo run --release --example launcher -- --environment browser --example client-movement -- \
  --client-spacing 9 --movement-speed 10
```

The same flags work in Native, WASI and WASI Threads. Values are fixed at startup.
Each movement input carries the run's motion settings so snapshot creation and
late-event replay use the same origin and speed without global configuration.

Bots send one input per second by default, all on the same source ticks starting
at one second. Their action times line up; independent random delays spread out
their deliveries. Keyboard actions keep the time of the actual key press.
A configured input frequency changes the shared tick interval.

Bot generation repeats North/East/South/West square cycles with a distinct phase
per client; it never sends Stop. Each side lasts one input period. The shared
graphical presentation compares original-timestamp ConTime replay on the left with
arrival-time-only movement on the right, using the same actual network handoffs.
Late turns repair the left path; the right path is never revised.
The player can still stop with Space, and all renderers retain Stop icons.
Arena walls still constrain movement.
