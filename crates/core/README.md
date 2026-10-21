# contime-core

`contime-core` connects ConTime's isolated subsystems through concrete protocol
and storage adapters. The root [`contime`](../../README.md) crate owns the running
engine, its configuration, startup, public operations, idle checks, and shutdown.
Core does not depend on the root crate or start threads.

## Responsibilities

- `Input` and `SharedEvent` connect consumer event identity, time, and snapshot
  routing to immutable shared ownership.
- Router and worker messages implement API, routing, and scheduling contracts.
- `RouterProcess` and `WorkerProcess` adapt those messages to the isolated runtime.
- History and checkpoint adapters preserve canonical event ordering, complete
  timestamp batches, replay hooks, and retained state.
- The admission coordinator owns fencing, rejection rules, progress observations,
  and the monotonic safe-to-prune boundary.

Core's `checkpoints` module exposes the application and retention hooks;
`lanes` re-exports the consumer application contracts. Live replay can publish
application effects, while query reconstruction and forwarding remain silent.
Admission coverage tracking is removed at insertion and never enters retained
history. Scheduling, canonical event storage, and checkpoint algorithms remain
in their respective specialized crates.

## Assembly boundary

The root constructs `RouterProcess` and `WorkerProcess` through `new`, connects
activity observers through `with_activity`, router flush requests through
`with_controls`, and worker progress callbacks through `with_coordination`.
`WorkerProcess::new(checkpoints, wrapper)` receives only settings its store uses;
retention stays with the admission coordinator. Process fields remain private. `SharedEvent::from` wraps an owned input; existing
API output traits construct protocol batches without exposing their fields.

`coordinate` runs the admission loop on the calling thread with
`CoordinatorSettings` (retention, worker count, and measurement interval). The
root supplies its channels and owns the thread. Core's private frontier and
admission rules remain here; this assembly interface does not duplicate them.

See the [running pipeline guide](../../docs/running-contime.md) for public
operations, completion boundaries, progress semantics, and performance notes.
The [ordered values example](../../examples/ordered_values.rs) uses the root API.

## Verification

```bash
cargo test -p contime-core
cargo test -p contime
```

The synchronous component regression in `tests/controlled/replay_frontier.rs`
is included in the library test binary to access private storage, frontier, and
admission code. It controls worker reports, resolution, and forwarding, including
a ten-worker case. Root integration tests cover the threaded pipeline.

```bash
cargo test -p contime-core --lib replay_frontier_tests
cargo bench -p contime --bench apply
cargo bench -p contime --bench query
cargo bench -p contime --bench advance
cargo bench -p contime --bench listen
```
