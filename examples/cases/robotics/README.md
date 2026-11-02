# Robotics: observations that finish late

Choose **Visualizer → 4 Robotics / delayed perception** in the window opened by
`cargo run --example launcher`. Native, WASI, WASI Threads and Browser use the
same sensor workload and shared Bevy renderer. Movement and service uptime are
separate examples.

A robot moves around fixed warehouse shelves. Wheel motion updates its estimated
position and heading ten times per second. During a slip patch the wheels report
too much motion and introduce a heading error. Camera localization jobs measure
position and heading at capture time, but processing takes time. Some jobs finish
in the opposite order from their captures.

Both maps receive and use every delivered observation:

- **From observation time:** ConTime applies a camera pose at its capture timestamp
  and replays subsequent body-relative motion and planning events.
- **From current time:** the comparison applies that same camera pose at the
  time its result becomes available. It does not repair the capture-time state.

The gray outlined robot is simulated physical truth. Colored robots and solid
trails are estimates. Dashed routes are proposals toward the aisle and inspection
goal, not commands executed by the physical robot. A drifting estimate may propose
backtracking to an entrance that is still ahead of the actual robot. Late evidence
changes the reconstructed estimate and subsequent proposal. Superseded paths and
proposals fade briefly. Position-error readouts compare each estimate to the same
query-time physical truth.

The sensor timeline uses **capture time / result available**: camera icons mark
captures, horizontal lines show processing latency, and end marks show availability.
Wheel ticks are 100 ms apart; stronger grid lines mark seconds. Upcoming jobs are
previewed. Delivered camera symbols travel into both maps before revised state is
shown. P and tab visibility pause the browser clock; Reset restarts the same script,
and Back returns to the visualizer menu.

## Application model

Start with [model.rs](model.rs). `SensorEvent` carries an immutable observation or
motion increment. `Robot` contains the estimator's persistent state, the latest
stateful maneuver proposal, event counts and a bounded trajectory. The optional
derives supply time access, IDs, creation and event registration. Application code
owns the estimation and planning rules.

Odometry describes motion through each 100 ms interval. Its distance uses the
preceding body orientation; its turn establishes the orientation for the following
interval. Camera captures align with interval boundaries. Same-time batches have
explicit phases: motion through T, camera observation at T, then planning at T.
Camera updates use fixed gains of 0.85 for position and 0.8 for wrapped heading.
Planning runs every 500 ms and stores both a maneuver and its proposed waypoints,
using only estimated state. Motion updates between planning ticks do not invent a
new route in the renderer. The proposal approaches, returns to, or enters the known aisle. This is a simplified complementary estimator and waypoint proposal,
not a general localization or path-planning library.

The [scenario](../../support/robotics/scenario.rs) alone knows physical truth. It
provides scripted motion and generates sensor measurements through simulator
callbacks. Estimation and planning cannot inspect truth. Camera pairs capture at
4/5, 8/9, ... seconds, with processing delays of 1.8/0.4 seconds respectively.
The physical route loops every 40 seconds; the slip patch repeats on that loop.

The [shared driver](../../support/robotics/driver.rs) collects worker status, queries
actual ConTime state, and checks it against an independent chronological fold of
all delivered evidence. The current-time comparison retains the same application
rules and changes only the effective timestamp of camera observations. Pending
and ungenerated inputs constrain retention advancement; old reference events are
compacted. Reports and trajectory history are bounded. Stop discards future jobs,
catches up captured inputs through the frozen stop horizon, flushes results in scheduled order, drains execution and validates the
final state. Finite runs use the same final-validation path.

## Boundaries

This demonstrates reusable historical replay, not a new sensor-fusion algorithm.
Correctly designed robotics estimators can handle delayed measurements too.
Synchronized, authoritative capture timestamps, a known static map and stable
sensor results are assumed. Measurement processing is scripted; no AI model runs.

Replay corrects an estimator from the observation timestamp forward. It is not
backward smoothing and does not guarantee physical truth. The robot has already
moved while perception was computing; reconstruction cannot undo that movement.
Both maps therefore share an identical scripted physical route. No proposal drives
hardware, and this example makes no safety or collision-avoidance guarantees.
