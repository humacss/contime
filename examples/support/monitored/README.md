# Shared simulator/monitor support

Used by native/WASI counters and hashes in both supported execution modes. Browser counters and hashes reuse the controller, simulator,
Manual backend and reference checks through a frame-budgeted adapter. Models and
simulator callbacks stay in their case folders. Monitoring, workload control and
presentation live in support; generic execution and host lifecycle live under
`../../environments`. ConTime itself is not modified.

| File | Responsibility |
| --- | --- |
| `host.rs` | Native diagnostic composition and terminal observer; delegates process lifecycle to the environment |
| `driver.rs` | Shared simulator loop, monitor-only feedback, canonical reference, validation and termination |
| `metrics.rs`, `wire.rs` | Presentation-neutral metrics and bounded WASI worker-report transport |
| `portable.rs` | Terminal-free composition for WASI and browser hosts |
| `reference.rs` | Canonical generated state, query-result checks and final validation |
| `backend.rs` | Monitor bridge and diagnostic interface around the environment execution resources |
| `control.rs` | Maximum probing and monitor-only window completion |
| `options.rs` | Startup-only shared host settings and validation |
| `sampling.rs` | Seeded snapshot-ID assignment and variable fanout helpers |
| `ui.rs` | Composes the example-owned `../ui/workers` presenter with hash rows and digest inspection |

Counter callbacks are in `examples/cases/counter/monitored/generation.rs`; hash callbacks
are in `examples/cases/chain_hash/generation.rs`. Each creates an event and then assigns
its identifying fields through `contime-simulator::EventStream`.

## Processing feedback

Maximum submits bounded source-event windows with a nominal 50 ms load setting.
A window is processed only when a monitor collection requested after its seal
reports its exact cumulative target-application insertions, accepted processing
targets at or beyond the window end, and no pending replay on every worker.
This proof assumes the host is the exclusive producer, IDs are unique,
destinations are distinct, and application logic produces no additional events.
Rejections or unavailable/saturated counters are fatal. Raw replay traversals
are never treated as unique completed events.

The next offered load rises by at least 20% after prompt completion, or probes
110% of observed throughput if larger; a window slower than 50 ms backs off to
90% of its observed throughput. Work is bounded at one million target
applications per window, conservatively sized using the configured maximum
fanout. Numeric rates remain paced and may retain a source-window ceiling.

The dashboard distinguishes target events/s, average monitor-confirmed completed
events/s since startup, and submitted events not yet confirmed. Completion is
window-granular: outstanding is a conservative bound, not an exact instantaneous
queue length. Monitoring delay and host work contribute to observed throughput.

## Canonical validation and reversed delivery

The host builds expected state from generated events sorted by timestamp and
ID, then reverses delivery groups for the hash example. Each hash group shares
a primary destination. Low fixed rates wait for a pair before generation when
capacity allows; one-event ceilings and final single slots are exceptions.

Actual target counts are recorded separately from source events, including
variable fanout. They are not inferred as events multiplied by the maximum.
The reference uses existing example `Example` hooks for creation, canonical
application, identity, count, value, and optional full digest.

Hash checks run at completed-window boundaries, up to `query_hz`. A pending
query freezes new generation and advancement, preserving both the canonical
prefix and its retention boundary. Query polling, monitoring, drawing, and input
are separate nonblocking steps. Query results never tune the controller or
release production credit. Every returned snapshot is validated, not just the
visible table rows. Missing nonempty histories, duplicates, unknown IDs, count,
value, or full-digest mismatch terminate the example.

Counters retain final-only snapshot validation; hashes also validate
periodically at completed-window boundaries. All examples validate the
final state after monitor-confirmed drain. Ctrl-C, SIGTERM, duration, and q/Esc
reach that path. Input is joined and the terminal restored before backend
shutdown and summary. The shared input and formatting modules are loaded once
through `examples/support/mod.rs`.

The hash viewport formats only visible rows. Tab selects worker/snapshot
scrolling; arrows, Page Up/Down and Home/End navigate. The footer shows both full
digests for the first visible snapshot; table digests are shortened, comparisons
use all 32 bytes.

## Shared execution boundary

Native counters and hashes use this host in both execution modes, as do
WASI counters and hashes. Hash hosts use simulator callbacks and monitor-only feedback. The launcher enables the native monitor
features and maps its existing settings through `Options::from_settings`, including skew, average/max fanout,
retention, checkpoints, query frequency, and numeric source-window limits.
Manual calls the existing `process()` once per loop turn, and validation also
steps it while awaiting its query callback. Work-ready steps do not sleep.
The adapter reserves four queue slots before submitting a simulator batch, so
advance/prune and validation requests can still fit. Explicit pruning is queued
after advancement about every 100 ms. Worker reports use the existing read-only
status capture; they never drive processing. Runtime delegates its original
submission, advancement, query and shutdown operations unchanged.
The manual-only example builds without enabling the threaded runtime.
Finite portable regression scenarios independently cover replay and retention.

Run focused verification from the repository root:

```sh
cargo test --all-features --example counter --example chain_hash
cargo test --example launcher --example chain_hash
cargo clippy --all-features --example counter --example chain_hash -- -D warnings
```

## WASI presentation boundary

The guest imports `driver.rs`, `backend.rs`, the controller, simulator callbacks,
and monitor collection. It does not depend on Ratatui, terminal input or the
launcher. Its observer writes metrics and immutable worker readings at 20 Hz in
launcher mode, or human-readable text at 1 Hz for direct/headless runs. Final
output is emitted only after the shared completion and reference checks pass.
The launcher decodes reports and uses the same worker and snapshot presenter as native hosts. Hash previews
contain at most 64 comparisons with full digests and an explicit total count;
all snapshots are validated in the guest. Collection timestamps belong to the guest clock; transport replays of
the same reading are ignored when updating cumulative averages.
