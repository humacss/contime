# contime-core

`contime-core` is the smallest complete apply-and-query composition of the isolated
ConTime subcrates. It owns the process topology and memory budget while
delegating API batching, deterministic routing, worker scheduling, canonical
event storage, checkpoint replay, lane application, and ownership accounting
to their specialized crates.

The crate does not depend on the root `contime` crate.

## Apply flow

```text
owned inputs
  -> conservative batch admission
  -> tracked shared events
  -> API batch
  -> shared router queue
  -> snapshot routes
  -> worker histories
  -> checkpoint replay
  -> lane application
  -> completion by sender closure
```

Consumers implement `Input`, the snapshot contracts re-exported through
`contime_core::checkpoints`, and their lane types through
`contime_core::lanes`. Accepted event allocations are tracked once. Router
fan-out clones only tracked pointers. Checkpoint snapshots are retained in
independently mutable tracked ownership and report size changes after replay.

The memory safety buffer is excluded from normal input admission. A batch that
does not fit is rejected as a whole with one `MemoryFull` result per event ID.
The first pass does not separately estimate unused capacity in internal event
collections; the configured buffer covers that implementation overhead.

## Query flow

Snapshot and event-history queries use the same runtime, router queues, and
worker queues as applies. Snapshot queries partition requested IDs across
workers and return only found boxed snapshots. Event queries target one
snapshot history and return cloned tracked handles over `[from, to)`. Receiver
closure signals that every affected worker has completed.

Query reconstruction is read-only: it does not modify retained checkpoints,
acknowledge event history, force replay, or change worker scheduling.

## Deferred scope

- Advance, horizon pruning, and memory reclamation policy
- Cross-worker transactional admission
- Lane macros

## End-to-end query benchmark snapshot

Local optimized results recorded on 2026-09-01. Each runtime is populated
before timing; the measured region contains one synchronous query from the
public API through router and worker response-channel closure.

| Routers | Workers | 1,000 snapshots | Snapshot throughput | 1,000 event handles | Event throughput |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 70.23 us | 14.24 M/s | 30.55 us | 32.73 M/s |
| 1 | 4 | 63.77 us | 15.68 M/s | 31.11 us | 32.15 M/s |
| 1 | 10 | 96.13 us | 10.40 M/s | 34.56 us | 28.94 M/s |
| 2 | 10 | 94.30 us | 10.60 M/s | 28.77 us | 34.75 M/s |

Snapshot queries partition 1,000 independent IDs and return 1,000 boxed exact
checkpoint clones. Event queries target one history and clone 1,000 tracked
handles over a half-open range. Additional workers only help the partitionable
snapshot case when the parallel work exceeds added channel and completion
overhead; a single-history event query always executes on one worker.

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

## Verification

Run unit tests:

```bash
cargo test --manifest-path crates/core/Cargo.toml
```

Run each inline unit benchmark:

```bash
cargo test --release --manifest-path crates/core/Cargo.toml \
  memory::tests::benchmark_memory -- --ignored --nocapture
cargo test --release --manifest-path crates/core/Cargo.toml \
  input::tests::benchmark_input -- --ignored --nocapture
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
  apply::tests::benchmark_apply -- --ignored --nocapture
cargo test --release --manifest-path crates/core/Cargo.toml \
  shutdown::tests::benchmark_shutdown -- --ignored --nocapture
```

Run the end-to-end send benchmark:

```bash
cargo bench --manifest-path crates/core/Cargo.toml --bench apply
cargo bench --manifest-path crates/core/Cargo.toml --bench query
```

Run only the topology matrix:

```bash
cargo bench --manifest-path crates/core/Cargo.toml --bench apply -- send_topology
```
