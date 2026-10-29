# contime-core

Shared application contracts and synchronous storage adapters for ConTime.
Core starts no threads and defines no channel-based request protocol. It does
not depend on Runtime, the router, the batcher, or distributed progress tracking.

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
time-based retention.
It does not route events or decide when processing should run.

The runtime wraps this storage to account for in-flight admission observations.
Those observations never enter retained history. Manual remains a generic
worker driver and may use this adapter or another implementation of the worker
storage contract. The WebAssembly and browser examples use the public manual
facade with this shared storage adapter.

The optional `monitor` feature enables worker counters and the native storage's
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

## Full benchmark run — 2026-10-28

Measured on Apple M3 Pro (11 logical CPUs), Rust 1.90.0, optimized Cargo bench
profile, with unpinned threads and normal desktop background activity. All groups
ran sequentially after compilation. Existing warm-up, sample-count and measurement
settings were retained; no shortened timing overrides were used. These are local
measurements, not production-capacity guarantees. The workspace lockfile uses
Criterion 0.5.1, Crossbeam channel 0.5.15 and Crossbeam utils 0.8.21.
Earlier dated results above
remain historical records and may use different fixtures or settings.

Tables report the **median time per benchmark iteration**, with its bootstrap
95% confidence interval. Rates are derived from that median only where the
benchmark declares a throughput unit; that unit may be source events, target
applications, snapshots, bytes, or bookkeeping calls. It is not automatically
ConTime's event-processing capacity. Thread-startup and allocation boundaries
remain those defined in each benchmark source.

Feature variants were measured in separate sequential runs, not interleaved
paired trials; changing background load can affect their differences.

Historical 0.1.0 feature sets (before monitoring enabled counting automatically):
`all-features` is the workspace feature union, including optional
worker counters; `default` disables optional counters; `status-no-counters`
enables Monitor's `runtime` feature without counters. Each table states the
actual sample count. Inline tests may contain multiple Criterion cases.

### `contime_core` · inline · all-features

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `core/history/1000_ordered_inserts` | 20.425 µs | 20.409 µs–20.455 µs | 100 | — |
| `core/wrapper/1000_events/one_batch` | 1.359 µs | 1.358 µs–1.361 µs | 100 | — |
