# contime-router

`contime-router` dispatches caller-owned inputs, historical queries, listener
collections, and coordination messages to worker queues. It owns no threads or
storage. Start with `Placement`, implement the input/output adapter traits, and
use `route` for apply-only work or `route_messages` for the unified protocol.

Dispatch success means messages were sent, not processed. A disconnected worker
can leave earlier worker sends in effect; routing provides no rollback. Inputs
move to their final route and clone only for additional routes. Reuse identical
placement and worker counts across every operation in a topology.

Horizon advancement is broadcast directly to every worker without snapshot
hashing or route-vector allocation. The final worker takes the original time
and completion handle; earlier workers receive clones.

The broadcast does not establish a barrier with apply batches being processed
by another router. With multiple routers, an earlier apply and a later advance
can therefore reach the same worker in the opposite order. The router crate
provides dispatch, not cross-router sequencing. Runtime coordinates safety through
router fences and worker reports. `Resolve` broadcasts the round identifier,
authorized horizon, and shared completion ownership to every worker. Completion
closes only after all recipients release the round and finish required forwarding;
Runtime serializes subsequent rounds against that completion.

`contime-router` receives complete input batches, deterministically maps each
snapshot route to a worker, and sends one final batch per affected worker.

The managed runtime uses this workspace member through its `RouterProcess`
adapter. The router itself has no dependency on `contime` or `contime-api`.
`route_messages` serves the running pipeline; the independent `route` helper
uses the same dispatch kernel for apply-only routing.

## Responsibilities

- Receive router-local input batches.
- Map snapshot IDs modulo the worker count, without hashing by default.
- Flatten directly into final worker vectors.
- Send one batch per affected worker.
- Partition snapshot queries into one message per affected worker.
- Partition each timestamped snapshot-listener collection into at most one
  message per affected worker, preserving its collection boundary.
- Route single-history event queries directly to one worker.
- Dispatch apply and both query kinds over one unified queue.

All routing functions take the same `Placement` value. `Placement::default()`
uses the full `u128` snapshot ID; `Placement::with_mapper(fn(u128) -> u128)`
lets a consumer supply a stable mapping or hash before modulus. Keep that
mapping identical for every router, apply, query, and listener in a topology.

The router is generic over input ownership. A single snapshot route moves the
existing input without cloning it; each additional snapshot route calls
`Clone` on the caller-selected input type. Workers receive exact
`{ snapshot_id, input }` records and never revisit or rehash snapshot IDs. The
router does not know whether cloning copies an owned value, increments a shared
pointer, or updates a tracked pointer.

Message ownership is also caller-selected. `RouteInputBatch` consumes the
router input, `RouteOutput` constructs each snapshot route, and `WorkerOutput`
constructs each affected worker's message. The crate's `InputBatch`,
`RoutedInput`, and `WorkerBatch` structs are optional default implementations.
An orchestrator may instead implement these traits on adapter types shared with
the adjacent API and worker boundaries.

## Exclusions

`route_messages` accepts an ordinary work receiver and a registration receiver
carrying `Sender<bool>` subscribers (`false` = idle, `true` = working).
Pass `crossbeam_channel::never()` when no activity subscriptions are needed.
The idle loop owns registrations and notifications, publishing working before
dequeue. The working loop drains and dispatches without activity management.
Work-message contents and worker sends are unchanged.


The crate owns no threads, workers, memory accounting, horizon policy or
state, rejection semantics, response waiting, cross-router barriers, or
recovery orchestration.

## Verification

Run the inline unit tests from the ConTime repository root:

```bash
cargo test --manifest-path crates/router/Cargo.toml --lib
```

Run the inline Criterion benchmark separately in release mode:

```bash
cargo test --manifest-path crates/router/Cargo.toml --release --lib benchmark_route -- --ignored --nocapture
cargo test --manifest-path crates/router/Cargo.toml --release --lib benchmark_snapshot_listener_routing -- --ignored --nocapture
```

Run the sustained integration benchmark:

```bash
cargo bench --manifest-path crates/router/Cargo.toml --bench router
cargo bench --manifest-path crates/router/Cargo.toml --bench query
```

Generate its CPU flame graph with optimized code and full symbols:

```bash
CARGO_PROFILE_BENCH_DEBUG=2 cargo bench --manifest-path crates/router/Cargo.toml --bench router -- 64_byte_events/8_workers/1_route --profile-time 10
```

Criterion writes the graph to
`target/criterion/router_100_batches_1000_inputs_64_byte_events_8_workers/1_route/profile/flamegraph.svg`
inside this crate.

## Benchmarks

The `router` benchmark covers routing with different worker counts, event sizes,
ownership strategies and fanout. The `query` benchmark measures snapshot-query
partitioning and event-query dispatch. Receivers remain alive without processing
worker messages, so these measure dispatch rather than application throughput.
Run the commands in Verification above. Compare matching fixtures and include
allocation and output-lifetime differences when interpreting results.
