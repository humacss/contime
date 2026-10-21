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

The repository root is a virtual workspace. Depend on `contime-core` for the
complete pipeline; there is no legacy root `contime` package.

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

Live processing uses `replay_event_batch`; queries and forwarding use only
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

The component-integration regression in `tests/controlled/replay_frontier.rs`
is included in the library test binary to access Core's private storage adapter,
frontier and admission function. Run it with
`cargo test --manifest-path crates/core/Cargo.toml --lib replay_frontier_tests`.
It drives real workers synchronously, including a ten-worker case, and controls
delivery of round resolution and forwarding. It covers reconstructed prefixes,
arrivals during and after measurement, silent forwarding, and eventual progress.
The separate `tests/replay_frontier.rs` retains threaded end-to-end coverage.

- Cross-worker transactional admission
- Lane macros

## Horizon advancement

The pruning frontier uses `contime-progress::MeasurementRound` for bounded
participant-report collection. Core retains admission rules, router fencing,
worker resolution, and its monotonic safe-to-prune boundary. The extracted
measurement has no authority to prune or reject events; a second observation
policy can reuse it without equating progress with pruning.

### Injected progress policy

`ConTime::start_with_progress(config, wrapper, policy)` installs a
`ProgressPolicy<Time>` in the existing admission coordinator. `start` still
installs no observer and preserves its existing public API.

The policy supplies `cutoff(advanced)` and receives
`observed(ProgressObservation { round, cutoff, before })`. Core caps the cutoff
at the processing target. `before` is an exclusive local observation, not a
monotonic commitment, a pruning permission, or proof of remote delivery.
Later accepted input, including internal feedback, may create earlier work.
Before routing a nonempty admitted batch, Core calls `policy.invalidated()` so
consumers can discard cached observations before insertion is acknowledged.
The default callback does nothing. It is called once per batch, not per event;
empty or wholly rejected batches do not invalidate. Subsequent observations
account for the admission using the existing round constraints. Inputs queued
outside Core still require accounting by the caller.

Observation and pruning share the same numbered router-fence/worker-report
round. The round measures to the larger cutoff, but pruning remains capped at
the retention boundary captured when that round started. Observers receive
only their own capped result. Admission during a round still constrains pruning
to the existing safe-to-prune boundary, as in the original pruning protocol.
Observation uses a separate reduction over the same fenced worker reports.
Each in-flight admission holds that conservative bound until all its target
snapshot stores report the actual replay bound after insertion. Covered routes
can then constrain observation by actual replay instead of the pruning boundary;
uncovered routes retain the fallback. Event timestamps alone never discharge
these constraints. This tracking is removed before events enter retained history
and is allocated only for admissions during an active observation round.

Startup, admission and advancement make observation eligible. Incomplete
measurements retry without another advance; completed ones sleep until another
admission or advancement. Both policies share `pruning_interval` as the minimum
spacing between measurement rounds. There is no periodic idle polling or new
thread. A future-only event beyond the observation cutoff does not require
advancement to complete that observation.

The existing worker resolution message resumes a round with the authorized
prune boundary. When that boundary has not changed, resumption does not prune.
No separate worker coordinator or queue tracker is installed.

Callbacks must be short and nonblocking: they run on the admission coordinator
and must not synchronously query or wait on the same Core. Consumers should
forward results through a channel or update their own observation state.
Changing only consumer-side configuration does not wake an idle coordinator;
the policy is reevaluated on the lifecycle changes listed above.

`tests/progress.rs` covers retained late input, feedback before an older report,
empty intervals, blocked processing, cutoff clamping and cross-worker feedback.
Stream group membership, delivery fencing and distributed commitment are still
the responsibility of Engine/Stream; no such transport is implemented here.

`advance_to` asynchronously raises the processing target and requests retention
of `history_retention` worth of time. The admission coordinator rejects new
external input before the requested horizon. Router fences and worker reports
establish a conservative safe boundary before any retained data is removed.
Accepted work and its causal outputs remain protected during that measurement.
Worker reports include possible reconstructed-prefix publications, not merely
incoming event timestamps. Workers pause live replay after reporting until their
round is resolved and authorized forwarding completes; queries and insertion
remain responsive. Any admission during measurement conservatively prevents
that round from advancing, since only storage can determine its replay prefix.
Every round is resolved, even without advancement, and resolution completion
precedes the next round. Continuous admission can therefore delay pruning.
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

## Snapshot-store integration measurements (2026-09-24)

Representative end-to-end measurements with Rust 1.95.0, one router and one
worker, 10 samples, 200 ms warmup and 1 s measurement. These are ballpark
measurements for that implementation date, not a guarantee for other workloads.

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
