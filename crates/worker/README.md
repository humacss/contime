# contime-worker

Use `SnapshotStore` to adapt history storage and `WorkInput` to adapt the
message protocol. `work_messages` drives the calling thread; `MessageWorker`
provides synchronous control without spawning threads. Public API rustdoc
specifies adapter ownership and completion contracts.

Snapshot queries include events at the requested time. Event-history queries
use `[from, to)`, including the lower bound and excluding the upper; missing
histories and empty results release response ownership without sending a result.
Insertion completion precedes replay, while pruning completion follows every
captured history’s forwarding and pruning hooks.

`contime-worker` owns incremental replay scheduling and the blocking message
receive loop. It depends on no other ConTime crate. Runtime supplies concrete
messages and wraps Core's shared storage adapter; Runtime owns the background
threads.

## Message flow

`work_messages(input, store_config, context, registrations, coordination)` runs
on the calling thread. Each history owns one implementation of `SnapshotStore`.
The worker accepts caller-selected message and input types through `WorkInput`,
`ApplyInput`, and `RouteInput`; `ApplyBatch` and `RoutedInput` are optional
transport containers.

The receive loop drains the messages ready at the start of a bounded receive
batch, then grants up to one replay step per received message. Each step handles
one snapshot through the next distinct timestamp bucket in the worker-wide
schedule or the processing target, whichever comes first. One store call may
process multiple event timestamps. Newly queued messages are checked between steps. Queries
remain serviceable, and continuous read traffic cannot starve replay.

An ordered set groups scheduled histories by timestamp, with snapshot ID
breaking equal-time ties. After each replay step, the store reports
its next replay timestamp; finished histories leave the schedule and future
work waits for its target. Stores must finish all replay through the inclusive
step endpoint, including a complete timestamp bucket. Late
insertions invalidate the complete same-time bucket and its suffix. Duplicate
insertions do not invalidate progress. Notifications flush between steps.

The target starts at the time type's default value. `Advance` raises it
monotonically; replay never processes timestamps beyond it. Retained future
inputs remain queryable and do not prevent idle. Snapshot reconstruction does
not publish effects, advance processing, or change scheduled work.

The worker has no independent replay-budget or deadline configuration. Its
message/replay interleaving is fixed by the incremental loop. Retention belongs
to admission coordination; worker pruning requires an explicit authorization.

## Completion, observation, and pruning

Apply completion acknowledges insertion, including any rejections. It does not
mean scheduled replay has finished. The activity receiver accepts independent
`Sender<bool>` subscriptions (`false` = idle, `true` = working). Registrations
are serviced in the idle loop; working is published before dequeue, and idle
only after replay, hooks, notifications, and queued pruning finish. Pass
`crossbeam_channel::never()` when no subscriptions are needed.

Optional `Coordination` reports possible replay work after a fence from every
router in a round. Duplicate and older fences are ignored. After reporting,
live replay pauses until matching resolution; messages and queries remain
serviceable. Resolution supplies the proven pruning horizon, which may be
unchanged. Pending work receives a processing opportunity between consecutive
reports so zero-interval measurements cannot starve replay.

Only `Prune` and `Resolve` authorize history removal. The worker passes the
admission horizon to every insertion, including while forwarding is queued.
Each pruning step forwards and prunes one history, then returns to message
handling. Completion and idle wait for the whole pass. The completed-horizon
callback runs only after every history and retention hook succeeds. A failed
hook or dropped completion sender never reports successful pruning.

## Worker status

`MessageWorker::status(worker_id)` captures worker orchestration gauges.
`WorkInputKind::Status` requests the same report through the message loop with
an explicitly supplied worker index and Crossbeam response sender. Sending the
report never blocks: a full or disconnected response channel drops it.

`WorkerStatus` distinguishes initialized history stores from listener-only
metadata, counts pending and currently due histories, and reports the accepted
processing target, admission horizon, queued pruning passes, and awaited
coordination round. Due work can still be blocked by pruning or coordination.
The horizon records accepted admission policy, not completed pruning. These
are history gauges, not retained-event counts.

The optional `counters` feature adds cumulative event-to-history application
counts to `WorkerStatus::counters`. It is disabled by default; the status field
always exists and is `None` in disabled builds. `WorkerCounters` is always
available. Insertions count accepted canonical changes, excluding duplicates and
rejections. Live replay counts raw canonical traversals, including repeated
prefix replay after late input, and excludes queries and retention forwarding.
These are per-history applications, so one input routed to several histories
contributes several insertions. Counts saturate at `u64::MAX` and retain a
`saturated` signal after overflow.

Custom stores can implement `SnapshotStore::process_until_counted` when enabled.
Its default calls normal processing and returns `None`; once any operation
cannot report a replay count, that worker's replay total remains unavailable.
No clock or rates are maintained by the worker.

Capture scans the history slots and due schedule prefix without calling stores,
reconstructing snapshots, processing events, pruning, or publishing hooks.
Reports from separate workers are not a single atomic system snapshot, and a
status request does not fence inputs still queued elsewhere in the topology.

## Synchronous driving

`MessageWorker::new` constructs the same state used by `work_messages` without
spawning threads. `MessageWorker::new_inline` constructs that state without
channel bindings, for owned state in an application host loop. Both constructors
use the same message handling, replay scheduling and pruning methods.
`handle` delivers a message, `step` processes a scheduled
snapshot, and `prune_step` forwards/prunes a history. A driver must preserve
normal priority: delivered messages, pending pruning, then replay. Runtime's
controlled frontier tests use this interface with real storage adapters.

## Verification and performance

Run from the repository root:

```sh
cargo test -p contime-worker
cargo clippy -p contime-worker --all-targets -- -D warnings
cargo bench -p contime-worker --bench messages
```

The `messages` benchmark checks observable insertion, processing, query,
notification, and pruning results with storage stubs before timing. It measures
worker overhead rather than end-to-end history or checkpoint performance.
Run timing suites serially; results depend on the fixture and hardware.

Source units are `work.rs` (receive loop and scheduler), `listen.rs` (replay
notifications), `query.rs` (store queries), and `types.rs` (transport and storage
contracts). `tests/incremental.rs` covers ordering, rewinds, queries, target
bounds, activity, fences, duplicates, pruning, and repeated mixed work.
