# contime-core

Shared application contracts and synchronous storage adapters for ConTime.
Core starts no threads and defines no channel-based request protocol. It does
not depend on Runtime, routing, or distributed progress tracking.

`Input` describes stable event identity, timestamps, and destination histories.
It has no `Send` or `Sync` requirement. The managed runtime adds those bounds
at its own boundary; thread-local events, snapshots, and application contexts
can use the shared storage directly.

## Application and storage

The `checkpoints` module exposes snapshot contracts, `ApplyEvents`, checkpoint
spacing, and `ApplyWrapper` hooks. Live replay uses the replay hook; historical
queries and forwarding use silent application. Retention can compact the moving
anchor through the retention hook. Optional typed application contracts are
re-exported through `lanes`.

`SharedEvent` shares immutable payload ownership across histories without
admission or channel metadata. `CheckpointStorage` implements the existing
worker storage contract over the canonical event history and snapshot store;
`CheckpointStorageConfig` supplies checkpoint spacing. Insertion failures carry
event IDs in transport-independent `InsertionRejection` records. The adapter
preserves duplicate detection, late-event replay, historical queries, and
time-based retention. It does not route events or decide when processing runs.

The runtime wraps this storage to account for in-flight admission observations.
Those observations never enter retained history. Applications may also use the
shared synchronous adapter directly with thread-local state and their own
scheduling of storage operations.

The optional `counters` feature enables worker counters and the native storage's
exact counted replay method. It counts raw canonical events traversed during each
live replay call, including rebuilt prefixes after late input and events skipped
by an application wrapper. Silent queries and retention forwarding are excluded.
Worker scheduling and the existing `process_until` method are unchanged.


## Verification

```sh
cargo +1.90.0 test -p contime-core
cargo +1.90.0 clippy -p contime-core --all-targets -- -D warnings
```

`tests/local_input.rs` covers thread-local event/application state, late events,
duplicates, silent query/forwarding behavior, and pruning through the shared
storage adapter. Snapshot and event storage algorithms remain in their own crates.
