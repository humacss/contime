# contime-core

`subscribe_pruned_horizon()` returns an independent receiver of completed history
pruning horizons. Registration sends the current horizon (initially the time
type's default); later values strictly increase only after every worker finishes
its pruning pass and retention hooks. This is distinct from requested advancement
and safe-to-prune permission. Consumers can drain these notifications without
blocking advancement. Shutdown closes the receiver; worker failure never counts
as successful pruning.

`contime-core` is the smallest complete apply, query, and advance composition of the isolated
ConTime subcrates. It owns the process topology while
delegating API batching, deterministic routing, worker scheduling, canonical
event storage, checkpoint replay, and lane application
to their specialized crates.

The crate does not depend on the root `contime` crate.

`ConTimeConfig::placement` replaces `router_seed`. Use `Placement::default()`
for snapshot-ID modulus routing, or `Placement::with_mapper` to provide a
deterministic mapping. Core supplies the same policy to all routers and all
operation kinds.

## Apply flow

```text
owned inputs
  -> shared events
  -> API batch
  -> admission coordinator
  -> shared router queue
  -> snapshot routes
  -> worker histories
  -> time-ordered checkpoint steps
  -> lane application
  -> shared rejection stream
```

Consumers implement `checkpoints::Event` for event time, `Input`
for event identity and snapshot routing, the snapshot contracts exposed through
`contime_core::checkpoints`, and their lane types through
`contime_core::lanes`. Router fan-out shares immutable events through ordinary
`Arc` ownership. Histories directly own their mutable checkpoints.

Core's `checkpoints` module defines the iterator-based application/hook interface
for its consumers. Each worker history owns one concrete `SnapshotStore`, created
from the first accepted event with an initial snapshot at the active horizon.
The snapshot store owns canonical insertion, invalidation, replay, forwarding,
and pruning. Core's event-history adapter preserves event-ID ordering within
each complete timestamp. Worker scheduling does not inspect the event history.

`EventBatch<'iterator, 'event, T, E>` and `ApplyBatch` borrow an iterator of
`&E`; they no longer expose slices. The separate lifetimes allow local filtering
and peeking without cloning events or collecting replay ranges. Consumers can
use `batch.events.map(...)` or `by_ref()` to consume the batch. A consumer may
stop early: the snapshot kernel drains the remaining canonical timestamp batch
before recording its time and count.

`history_event_count` on Core's application context is the count **before** the
current canonical batch. All effective partitions see that preceding count;
the snapshot kernel increments it after application. Consumers must not treat
it as the resulting count or use it as a newly generated publication identity.

Live processing and forwarding use `replay_event_batch`; queries use only
`apply_event_batch` and never publish live effects. Forwarding additionally calls
`retain_snapshot` after consumed timestamps and at the horizon's predecessor.
That hook may compact consumer data without changing observable state.

Timestamp types implement both `checkpoints::Timestamp` (checked immediate
predecessor) and `contime_worker::AdvanceTime` (retention subtraction). A local
wrapper is required for primitive integers because these traits belong to
different crates. Tests and benchmarks define their own wrappers; automatic
macro generation is deferred. For example:

```rust
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
struct Time(u64);
impl contime_core::checkpoints::Timestamp for Time {
    fn previous(&self) -> Option<Self> { self.0.checked_sub(1).map(Self) }
}
impl contime_worker::AdvanceTime for Time {
    fn saturating_sub(&self, retention: &Self) -> Self {
        Self(self.0.saturating_sub(retention.0))
    }
}
```

`apply` enqueues external inputs and returns without waiting. Rejections are
received through `errors()`; use `wait_until_idle` when a test needs processing
to finish. The processing target starts at the time type's default value and
is moved forward by `advance_to`. Future inputs remain available to queries
but do not publish application effects until their time is reached.

Memory accounting and budget rejection have been removed. Their last committed
implementation is available at `085b00c44d12b040d9f80f3f7a74a8341d2b0644`.
History retention still prunes old events and checkpoints; actual process memory
must be monitored externally. There is no built-in memory cap.

## Query flow

Snapshot and event-history queries use the same runtime, router queues, and
worker queues as applies. Snapshot queries partition requested IDs across
workers and return only found boxed snapshots. Event queries target one
snapshot history and return cloned shared handles over `[from, to)`. Receiver
closure signals that every affected worker has completed.

Query reconstruction is read-only: it does not modify retained checkpoints,
acknowledge event history, force replay, or change worker scheduling.

## Explicit idle wait

`wait_until_idle(Duration)` waits until queued and active work has finished,
returning `Ok(())`, `IdleError::Timeout`, or an immediate
`IdleError::ComponentStopped` if a router/worker exits. For tests, stop submitting work, wait,
then query and assert. Waiting does not advance time or request pruning; it
also waits for already-requested safe pruning. Future-only history does not
prevent idle. A timeout stops waiting without cancelling processing.

## Deferred scope

- Cross-worker transactional admission
- Lane macros

## Horizon advancement

`advance_to` asynchronously raises the processing target and requests retention
of `history_retention` worth of time. The admission coordinator rejects new
external input before the requested horizon. Router fences and worker reports
establish a conservative safe boundary before any retained data is removed.
Accepted work and its causal outputs remain protected during that measurement.
Forwarding establishes a checkpoint at the horizon's predecessor; pruning
preserves it and keeps events exactly at the horizon. Snapshot queries before
the completed store horizon return no snapshot, never a stale anchor. The public
multi-snapshot query API continues to return only found snapshots, rather than
introducing a new per-snapshot error response.

`ConTimeConfig::pruning_interval` sets the minimum wall-clock spacing between
safe-pruning measurement rounds. Use 100 ms for the previous cadence, or
`Duration::ZERO` to start the next needed round immediately. Zero removes only
the timer delay: router fences and all worker reports are still required.
The `advance` benchmark uses zero, excludes fixture setup and teardown, and
verifies retained events and anchor state after the measured idle wait.

`apply_internal(source_time, inputs)` is reserved for causal outputs submitted
before an active application returns. It enforces the proven safe boundary
and rejects output earlier than its source time. Detached producers cannot
use this contract. Queries do not establish submission or replay barriers.

### Historical benchmark baseline

The figures below predate memory-accounting removal, incremental scheduling and the admission coordinator;
they are not measurements of the current implementation.

Local optimized advancement-only results for 1,000 histories on 2026-09-01:

| Routers | Workers | Clean prune | Anchor materialization | Forced replay |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 328.9 us | 664.2 us | 1.471 ms |
| 1 | 4 | 291.0 us | 426.7 us | 1.628 ms |
| 1 | 10 | 1.202 ms | 891.7 us | 1.857 ms |
| 2 | 10 | 1.068 ms | 406.9 us | 879.8 us |

Fixtures contain 1,000 histories and are prepared outside the timed region.
Each sample asserts that tracked memory decreases. The dirty multi-router
fixture first confirms through read-only queries that all histories reached
their workers, preserving the intended dirty-replay workload without adding a
production ordering barrier. These thread-sensitive figures showed broad
variance and should be treated as local reference points, not scaling claims.

## End-to-end query benchmark snapshot

Local optimized results recorded on 2026-09-01. Each runtime is populated
before timing; the measured region contains one synchronous query from the
public API through router and worker response-channel closure. Point estimates
are Criterion means.

Snapshot queries partition independent IDs and return boxed exact-checkpoint
clones:

| Routers | Workers | Results | Latency | Per snapshot | Throughput |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 1 | 19.101 us | 19.101 us | 52.354 K/s |
| 1 | 1 | 100 | 27.238 us | 272.38 ns | 3.6713 M/s |
| 1 | 1 | 1,000 | 65.376 us | 65.376 ns | 15.296 M/s |
| 1 | 4 | 1 | 20.323 us | 20.323 us | 49.205 K/s |
| 1 | 4 | 100 | 45.749 us | 457.49 ns | 2.1858 M/s |
| 1 | 4 | 1,000 | 67.559 us | 67.559 ns | 14.802 M/s |
| 1 | 10 | 1 | 19.438 us | 19.438 us | 51.446 K/s |
| 1 | 10 | 100 | 67.317 us | 673.17 ns | 1.4855 M/s |
| 1 | 10 | 1,000 | 96.442 us | 96.442 ns | 10.369 M/s |
| 2 | 10 | 1 | 19.022 us | 19.022 us | 52.572 K/s |
| 2 | 10 | 100 | 64.582 us | 645.82 ns | 1.5484 M/s |
| 2 | 10 | 1,000 | 96.037 us | 96.037 ns | 10.413 M/s |

Event queries target one history and clone tracked handles over a half-open
range:

| Routers | Workers | Results | Latency | Per event | Throughput |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 1 | 17.900 us | 17.900 us | 55.865 K/s |
| 1 | 1 | 100 | 22.110 us | 221.10 ns | 4.5228 M/s |
| 1 | 1 | 1,000 | 31.679 us | 31.679 ns | 31.566 M/s |
| 1 | 4 | 1 | 19.736 us | 19.736 us | 50.669 K/s |
| 1 | 4 | 100 | 21.704 us | 217.04 ns | 4.6075 M/s |
| 1 | 4 | 1,000 | 31.912 us | 31.912 ns | 31.336 M/s |
| 1 | 10 | 1 | 18.435 us | 18.435 us | 54.245 K/s |
| 1 | 10 | 100 | 22.142 us | 221.42 ns | 4.5164 M/s |
| 1 | 10 | 1,000 | 32.224 us | 32.224 ns | 31.033 M/s |
| 2 | 10 | 1 | 17.462 us | 17.462 us | 57.267 K/s |
| 2 | 10 | 100 | 21.619 us | 216.19 ns | 4.6257 M/s |
| 2 | 10 | 1,000 | 30.556 us | 30.556 ns | 32.726 M/s |

The one-result measurements expose roughly 17-20 us of fixed synchronous
round-trip cost. Snapshot queries can fan out across workers, but at these
sizes the extra worker messages and response coordination cost more than the
parallel checkpoint cloning saves. A single-history event query always runs on
one worker, so its throughput is largely topology-independent.

## Unit benchmark snapshot

Local optimized Criterion results recorded on 2026-08-31:

| Unit | Work | Point estimate | Per item |
| --- | ---: | ---: | ---: |
| Memory | 1,000 increases + 1,000 decreases | 5.4663 us | 2.733 ns/delta |
| Input | Track 1,000 owned events | 16.593 us | 16.593 ns/event |
| Message | Construct 1,000 routes and one worker batch | 670.95 ns | 0.671 ns/route |
| History | Insert 1,000 ordered tracked events | 25.876 us | 25.876 ns/event |
| Checkpoint | Replay 1,000 events at one timestamp | 7.4861 us | 7.486 ns/event |
| Router | Route 1,000 events to one worker | 2.7390 us | 2.739 ns/route |
| Worker | Insert, schedule, replay, and complete 1,000 events | 64.331 us | 64.331 ns/event |
| Send | Prepare and forward 1,000 inputs | 15.222 us | 15.222 ns/event |
| Apply rejection | Prepare, return, and collect 1,000 over-budget rejections | 23.534 us | 23.534 ns/input |
| Start | Start one router and one worker | 10.597 us | — |
| Shutdown | Join one router and one worker | 23.352 us | — |

### Snapshot replay listeners

`send_listen_snapshots` forwards one watched timestamp, a snapshot-ID set, and
a consumer-owned notification sender without waiting. The router preserves the
registration as one collection per affected worker. Each owning worker emits
one batched `SnapshotListenerMessage::Registered`, then at most one batched
`SnapshotListenerMessage::Replayed` for that collection after a worker replay
pass. A snapshot is included when its replay began at or before the watched
timestamp. Consumers drop their receiver when finished; subsequent failed
sends remove the collection lazily.

Listener unit results recorded on 2026-09-01:

| Unit | Work | Point estimate | Amortized |
| --- | ---: | ---: | ---: |
| API | Forward timestamp + 1,000 snapshot IDs | 205.28 ns | 0.205 ns/ID |
| Router | Route 1,000 IDs to one worker | 2.1067 us | 2.107 ns/ID |
| Router | Route 1,000 IDs across eight workers | 3.4233 us | 3.423 ns/ID |
| Worker | Register one collection with 1,000 IDs | 58.069 us | 58.069 ns/ID |
| Worker | Replay check with no collections | 2.1286 ns | 2.129 ns/replay |
| Worker | Replay check with one nonmatching collection | 6.2251 ns | 6.225 ns/replay |
| Worker | Accumulate + flush 100 matching IDs | 994.68 ns | 9.947 ns/ID |
| Worker | Accumulate + flush 1,000 matching IDs | 8.7402 us | 8.740 ns/ID |
| Core adapter | Emit one one-ID replay batch | 43.585 ns | 43.585 ns/message |

End-to-end results recorded on 2026-09-02 use long-lived warmed runtimes. Every
sample asynchronously sends 100 batches of 1,000 events (100,000 total); event
construction and listener registration are outside timing. Baseline and
enabled cases are otherwise identical, and enabled cases drain one notification
per affected worker replay batch. `Delta` is enabled minus baseline using the
Criterion point estimates.

| Routers | Workers | Listened snapshots | Baseline | Enabled | Delta | Baseline events/s | Enabled events/s |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 1 | 76.510 ms | 73.851 ms | -2.659 ms | 1.3070 M | 1.3541 M |
| 1 | 1 | 100 | 82.765 ms | 83.075 ms | +0.310 ms | 1.2082 M | 1.2037 M |
| 1 | 1 | 1,000 | 98.408 ms | 100.23 ms | +1.822 ms | 1.0162 M | 997.74 K |
| 2 | 4 | 1 | 54.042 ms | 54.042 ms | 0.000 ms | 1.8504 M | 1.8504 M |
| 2 | 4 | 100 | 63.193 ms | 63.432 ms | +0.239 ms | 1.5825 M | 1.5765 M |
| 2 | 4 | 1,000 | 72.025 ms | 72.250 ms | +0.225 ms | 1.3884 M | 1.3841 M |

The negative one-snapshot delta and broad multi-worker confidence intervals
show ordinary thread-scheduling noise rather than a speedup. The useful signal
is that listener overhead is mostly within measurement variance; the clearest
case, 1,000 listened snapshots on one worker, adds about 18.2 ns per event.

Disconnected listeners remain stored until their snapshot replays again.
There is no explicit listener identity or removal command.

The worker measurement intentionally includes all worker-owned work after its
batch is already available: snapshot lookup, canonical insertion, scheduling,
checkpoint replay, lane-independent snapshot application, completion, and the
worker loop's clean termination. It excludes API admission and routing. The
start benchmark times only process/channel startup and performs shutdown
outside its accumulated measurement.

The apply-rejection benchmark exercises the local memory-rejection branch and
does not enter the runtime. It includes sending and collecting all 1,000
rejection messages through the synchronous API. End-to-end send throughput
remains deliberately separate from these overlapping unit measurements.

## End-to-end Send benchmark snapshot

Local optimized Criterion results recorded on 2026-08-31 for one hot router,
one hot worker, one snapshot route per event, and 1,000 total successful events
per sample:

| Batches | Events per batch | Total latency | Per event | Throughput | Speedup |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1,000 | 1 | 754.08 us | 754.08 ns | 1.3261 M events/s | 1.00x |
| 100 | 10 | 143.76 us | 143.76 ns | 6.9559 M events/s | 5.25x |
| 10 | 100 | 84.606 us | 84.606 ns | 11.819 M events/s | 8.91x |
| 1 | 1,000 | 94.436 us | 94.436 ns | 10.589 M events/s | 7.98x |

Each measured workload repetition starts a fresh runtime and submits one
warm-up workload outside the timed region. The measured workload owns one
rejection channel for all batches, clones its sender once per batch before
timing, and drops the original sender. The timed region calls `ConTime::send`
for every batch and then drains (and ignores) rejection messages until the
receiver closes. Closure proves that every downstream sender clone has been
dropped after processing. Shutdown remains outside the measured region.

The timed path therefore includes memory admission and tracking, API
submission, channel handoff, routing, history insertion, scheduling,
checkpoint replay, and snapshot application without imposing one synchronous
round trip per batch. Batches of 100 are slightly faster than one batch of
1,000 here because ten queued batches let router and worker stages overlap;
very small batches eventually lose that benefit to per-batch channel and API
overhead.

## Send topology benchmark snapshot

The topology benchmark queues ten batches and assigns exactly 1,000 events to
each worker. Snapshot IDs are discovered through the real seeded router before
timing, so every worker receives an equal workload. Every batch/worker pair
uses a distinct snapshot, avoiding replay contention on one shared history.
Like the batch benchmark, the workload uses one rejection channel, clones its
sender once per batch, and finishes when the receiver closes.

| Routers | Workers | Batches | Total events | Total latency | Per event | Aggregate throughput |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 10 | 1,000 | 77.523 us | 77.523 ns | 12.899 M events/s |
| 1 | 2 | 10 | 2,000 | 114.82 us | 57.410 ns | 17.419 M events/s |
| 1 | 4 | 10 | 4,000 | 162.06 us | 40.515 ns | 24.682 M events/s |
| 1 | 8 | 10 | 8,000 | 275.61 us | 34.451 ns | 29.027 M events/s |
| 1 | 10 | 10 | 10,000 | 305.73 us | 30.573 ns | 32.709 M events/s |
| 2 | 10 | 10 | 10,000 | 311.41 us | 31.141 ns | 32.112 M events/s |

Two workers provide 1.35x the one-worker throughput, four provide 1.91x, eight
provide 2.25x, and ten provide 2.54x. Scaling is sublinear because routing,
shared-memory accounting, allocation, and channel traffic remain shared work.

At ten workers, the confidence intervals for one and two routers overlap. The
second router does not improve this workload: one router already feeds the ten
workers faster than they consume the routed batches.

## Snapshot-store integration measurements (2026-09-24)

Representative end-to-end measurements with Rust 1.95.0, one router and one
worker, 10 samples, 200 ms warmup and 1 s measurement. These are ballpark
measurements, not an apples-to-apples comparison with the historical tables above.

| Operation | Total time | Throughput | Time per item |
| --- | ---: | ---: | ---: |
| Send and process one batch of 1,000 events | 111.98 us | 8.930 M events/s | 111.98 ns/event |
| Query 1,000 snapshots | 78.512 us | 12.737 M snapshots/s | 78.512 ns/snapshot |
| Advance and prune 1,000 clean histories | 93.033 us | 10.749 M histories/s | 93.033 ns/history |
| Advance and prune 1,000 histories with intermediate checkpoints | 121.92 us | 8.202 M histories/s | 121.92 ns/history |
| Process, advance and prune 1,000 dirty histories | 371.10 us | 2.695 M histories/s | 371.10 ns/history |

Advance measurements use a zero pruning interval and wait for completed work.
Fixture setup, verification queries and shutdown are outside their timing.
Query throughput counts returned snapshots, and advance throughput counts
histories, not event applications. The dirty advance case includes processing
previously admitted events; it is not a pure pruning measurement.

## Verification

Run unit tests:

```bash
cargo test --manifest-path crates/core/Cargo.toml
```

Run each inline unit benchmark:

```bash
cargo test --release --manifest-path crates/core/Cargo.toml \
  message::tests::benchmark_message -- --ignored --nocapture
cargo test --release --manifest-path crates/core/Cargo.toml \
  history::tests::benchmark_history -- --ignored --nocapture
cargo test --release --manifest-path crates/core/Cargo.toml \
  checkpoint::tests::benchmark_checkpoint -- --ignored --nocapture
cargo test --release --manifest-path crates/core/Cargo.toml \
  router::tests::benchmark_router -- --ignored --nocapture
cargo test --release --manifest-path crates/core/Cargo.toml \
  worker::tests::benchmark_worker -- --ignored --nocapture
cargo test --release --manifest-path crates/core/Cargo.toml \
  send::tests::benchmark_send -- --ignored --nocapture
cargo test --release --manifest-path crates/core/Cargo.toml \
  start::tests::benchmark_start -- --ignored --nocapture
cargo test --release --manifest-path crates/core/Cargo.toml \
  shutdown::tests::benchmark_shutdown -- --ignored --nocapture
cargo test --release --manifest-path crates/core/Cargo.toml \
  listen::tests::benchmark_listener_notification -- --ignored --nocapture
```

Run the end-to-end send benchmark:

```bash
cargo bench --manifest-path crates/core/Cargo.toml --bench apply
cargo bench --manifest-path crates/core/Cargo.toml --bench query
cargo bench --manifest-path crates/core/Cargo.toml --bench listen
```

Run only the topology matrix:

```bash
cargo bench --manifest-path crates/core/Cargo.toml --bench apply -- send_topology
```
