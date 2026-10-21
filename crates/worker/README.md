# contime-worker

`contime-worker` owns incremental replay scheduling and the blocking message
receive loop. It depends on no other ConTime crate. Core supplies concrete
messages and a `SnapshotStore` adapter; the root orchestrator owns the threads.

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

An ordered set groups pending work by timestamp. Equal-time entries that still
need that timestamp run before completed entries, then by snapshot ID. Late
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

## Synchronous driving

`MessageWorker::new` constructs the same state used by `work_messages` without
spawning threads. `handle` delivers a message, `step` processes a scheduled
snapshot, and `prune_step` forwards/prunes a history. A driver must preserve
normal priority: delivered messages, pending pruning, then replay. Core's
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
Run timing suites serially. Earlier measurements, including the retired batch
worker, are preserved in the [dated baseline record](../../docs/benchmarks/2026-09-24-worker-baselines.md).

Source units are `work.rs` (receive loop and scheduler), `listen.rs` (replay
notifications), `query.rs` (store queries), and `types.rs` (transport and storage
contracts). `tests/incremental.rs` covers ordering, rewinds, queries, target
bounds, activity, fences, duplicates, pruning, and repeated mixed work.
