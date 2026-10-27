# contime-manual

Create a caller-driven ConTime instance by driving one existing `contime-worker::MessageWorker` on the calling thread. No
router, coordinator thread, alternate replay engine, or snapshot task is used.
The worker owns all histories, replay scheduling, queries, and pruning.

`ManualConTime<M, S>` accepts the existing worker message and storage contracts.
Create it with a positive queue capacity, storage configuration, and application
context. `enqueue` returns the original message when full. `process()` makes
one unit of progress and returns whether work was done. It handles at most one
complete worker operation or pruning-barrier transition before returning to the
host. One step may apply several events. `process_with_clock` repeatedly calls
that same step within a maximum work/time allowance, returning early when no work
is ready. Neither method sleeps or waits for input; the helper never fills an
unused allowance with waiting.
A bounded ready-message batch gets a replay/pruning opportunity even with a
one-call-per-frame limit. There are no additional Send or Sync requirements.

A query finishes in one call; a replay call may process multiple timestamps;
a prune call forwards and physically prunes one history. None is interrupted.
The budgeted helper checks time between calls, so one operation can exceed its
allowance. Only that helper requires a monotonic real clock. With `process()`,
the host simply runs its other tasks between processing calls.

Pruning is an explicit queue barrier. Its horizon must not exceed the accepted
processing target. Earlier replay finishes before pruning is authorized, and
later messages wait until the entire prune pass finishes. Invalid horizons return
`ProcessError` instead of advancing history or waiting indefinitely for a later
Advance message. A target acknowledgement means acceptance, not completed replay.

Reports count worker calls, elapsed time, and queued messages. `NoReadyWork`
means no work is currently runnable at the target; future events may remain.
`BudgetExhausted` makes no assertion about remaining replay. Queue capacity does
not bound history or response storage. Response handlers and callbacks must not
block on the same driver. Panics propagate; discard the driver after a caught
application/storage panic, matching the existing worker's failure model.

For ordinary applications, the root crate provides a complete
`contime::manual::ManualConTime` instance with
`default-features = false, features = ["manual"]`. It combines this driver,
Core's storage, and local request adapters so callers implement only the ordinary
event and snapshot contracts. `apply`, `query_at`, `advance_to`, and explicit
`prune_before` enqueue work; call `process()` from your host loop to drive it,
or use `process_with_clock` when allocating a time/work allowance. Query callbacks run
during processing and may capture thread-local state. `take_rejections` drains
insertion failures. Subscriptions are not exposed by that convenience API.

The [manual counter example](../../examples/cases/counter/manual.rs) demonstrates a
host loop using only the root crate. Run it with:

```sh
cargo run --no-default-features --features manual --example manual_counter
```

Advanced callers can still use this crate directly, or the root API's
`contime::manual::WorkerDriver` re-export, with their own protocol and storage.
Neither entry point pulls in the managed runtime.

This crate has no snapshot/event dependency. The WebAssembly example uses the
public manual facade, which composes the request and storage adapters; the
browser host reuses that example. The worker's
channel-free constructor permits safe ownership in a browser export or application
resource without leaked receivers or self-referential storage.

```sh
cargo +1.90.0 test -p contime-manual -p contime-worker
cargo +1.90.0 clippy -p contime-manual -p contime-worker --all-targets -- -D warnings
```

For wasm32-unknown-unknown, enable the browser entropy backend required by the
worker's AHash dependency. The browser example contains that target configuration.

`status(worker_id)` reads the existing worker gauges and optional counters without
processing work. The root Manual instance exposes it as `worker_status()` with
worker ID zero. Reading status scans history slots and the due schedule prefix;
it neither reconstructs snapshots nor changes replay or pruning.
