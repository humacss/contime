# contime-runtime

`contime-runtime` owns the managed ConTime instance, admission coordination,
protocol adapters, background threads, and shutdown. Shared application and
storage contracts live in `contime-core`.

Use `ConTime::start` with `ConTimeConfig` to start a managed instance. Public
operations submit inputs, advance processing, query retained state, and register
listeners. `shutdown` closes admission and joins the owned topology. Inputs,
application state, and wrappers must satisfy the managed thread boundaries.
`ThreadInput` adds `Send + Sync` for events and timestamps to Core's `Input`
contract; snapshot state and application wrappers must be sendable as well.
`Runtime<I, S, W>` is an alias for the managed `ConTime<I, S, W>` instance.

The low-level topology is available separately as `ThreadTopology`; the guide
below describes that generic thread and channel owner.

Start with the `Router` and `Worker` traits, then pass owned nonempty process
collections to `ThreadTopology::start`. This is a generic thread topology: its message
types and process behavior are supplied by the caller.

Submission acknowledges enqueueing. `shutdown` closes the owned sender and
joins every component, preserving errors and panics in `ShutdownReport`. Drop
external input-sender clones before calling it. Process implementations must
release channels and return for shutdown to finish. Dropping `ThreadTopology` without
calling `shutdown` detaches join handles and does not wait for completion.

`ThreadTopology` is the isolated process-local execution topology for
ConTime's apply path. It starts router and worker threads, connects them with
Crossbeam channels, exposes one shared opaque router input boundary, and joins the complete
topology during shutdown.

The topology declares local `Router` and `Worker` traits. Managed process
adapters in this crate implement those contracts and connect the shared Core
storage adapter to the router and worker crates.

## Internal transport batching

The admission coordinator uses `contime-batcher` after admission and before
routing. It splits large submissions and combines adjacent requests already in
the queue, without waiting for more arrivals. Gathering stops at control messages
and is bounded by request count and CPU time. Routing preserves each request's
rejection destination and insertion-completion ownership, including across
multiple workers. Internal packet boundaries do not change timestamps or
canonical replay order.

## Worker observations

`request_worker_status()` queues a read-only request directly to each worker and
returns a dedicated response stream. `worker_count()` gives the expected number
of reports. Each report describes its worker when that worker handles the request;
it is not a synchronized global snapshot or an admission/replay completion fence.

Reports distinguish actual history stores from listener-only metadata, pending
histories from histories due at the processing target, and the admission horizon
from completed pruning. Due work may still be blocked by coordination. Counts
refer to histories, not events or snapshot reconstructions. Capturing a report
scans existing history slots and due schedule entries without reading stored
application data, replaying events, advancing time, or adding per-event counters.

The `contime-monitor` runtime adapter collects complete worker sets without
blocking and exposes immutable readings separately from any presentation layer.
Dropping a response receiver never blocks a worker. Status requests share normal
worker queues, so collection can be delayed by queued work.

## Topology

- Every router competes for work from one shared input receiver.
- Every worker has one private input receiver.
- Every router receives senders for the complete worker set.
- The runtime treats router input and worker input as opaque generic values.
- The caller supplies the complete router and worker instance collections.
- Router and worker collections must both be nonempty.

The generic topology owns thread and channel lifecycle only. It does not inspect,
clone, route, apply, replay, or otherwise interpret messages.

`queue_checks()` exposes read-only emptiness checks created with the queues:
one for the shared router queue and one per worker. They retain only weak
receiver references, so observation cannot keep a stopped queue connected.
A stopped queue returns false. Queue emptiness alone does not establish idle;
the caller must also account for active processing.

The shared receiver guarantees that each input message is taken by exactly one
router; it does not guarantee that several routers finish dispatching messages
in receive order. Each worker channel preserves the order in which that worker
receives messages from its router producers, but there is no fence spanning all
router producers. Higher layers must wait for completion before submitting an
operation that must happen after earlier work, or introduce an explicit
ordering barrier.

## Lifecycle

`ThreadTopology::start` validates the supplied process collections, creates the channels, starts all
workers, and then starts all routers. Threads receive deterministic names:
`contime-worker-{index}` and `contime-router-{index}`.

If a thread cannot be spawned, startup closes the partial topology and joins
every thread already started before returning `StartError::ThreadSpawn`.

A running `ThreadTopology` offers `send(input)` and borrowed access to its shared
router input sender. The first available router receives each message. Callers may clone the sender, but explicit shutdown cannot finish until
all external clones have been dropped.

`ThreadTopology::shutdown` drops the runtime-owned input sender, joins every router,
then joins every worker after the router-owned worker senders have closed. Its
report preserves one ordered outcome per thread and distinguishes completion,
a returned implementation error, and a panic. The runtime observes failures
but does not restart or recover failed threads in this version.

The generic topology leaves queries, advancement, retention, routing and replay
to its process adapters. The managed instance assembles those adapters. Neither
layer automatically restarts failed threads.

## Verification

Run the inline unit tests from the ConTime repository root:

```bash
cargo test --manifest-path crates/runtime/Cargo.toml
```

Run the warm end-to-end throughput benchmark:

```bash
cargo bench --manifest-path crates/runtime/Cargo.toml --bench runtime
```

Run the channel-stage and batch-size diagnostics:

```bash
cargo bench --manifest-path crates/runtime/Cargo.toml --bench breakdown
cargo bench --manifest-path crates/runtime/Cargo.toml --bench batching
```

Generate a ten-second flamegraph for one topology:

```bash
CARGO_PROFILE_BENCH_DEBUG=2 cargo bench \
  --manifest-path crates/runtime/Cargo.toml --bench runtime -- \
  'runtime/1000_inputs/topology/1_routers_1_workers' --profile-time 10
```

## Benchmark Snapshot

> These measurements predate the 2026-08-31 shared-router-queue API change.
> The benchmark sources compile against the new topology, but the end-to-end
> measurements have intentionally not been rerun during the core unit pass.

Local release-mode Criterion results recorded on 2026-08-29:

| Topology | 1,000 inputs | ns/input | Inputs/s |
| --- | ---: | ---: | ---: |
| 1 router, 1 worker | 49.761 us | 49.761 | 20.096 million |
| 2 routers, 4 workers | 65.612 us | 65.612 | 15.241 million |

The runtime is started once before timing and remains hot for the complete
Criterion run. Every timed iteration sends 1,000 benchmark events through the
shared router sender. The receiving router forwards
the event through the worker sender selected by `event.worker_index`. There is
no hash, modulo, runtime trait lookup, or runtime message inspection in the
timed path. Criterion's untimed setup creates the completion channel and
clones its sender into every prepared input. The measured routine only sends
those inputs and waits for the receiver to disconnect after workers process
them and drop every sender clone. No acknowledgement messages are sent.

Both cases include two Crossbeam channel hops,
the router and worker receive loops, and final queue-drain observation. They
exclude startup, shutdown, real snapshot routing, event storage, replay, lane
projection and filtering, checkpoint work, API rejection collection, and
every sibling ConTime crate.

The process-wide flamegraphs place the dominant sampled work in Crossbeam's
send/receive, backoff, park, and unpark paths. Allocation is a minor sampled
cost. This means the current tens-of-nanoseconds floor is channel scheduling
and synchronization, not router-index calculation inside the runtime.

## Batch Size Results

The batching benchmark gives every worker one prepared shared-slice batch of
the requested size. Total events therefore equal `events_per_worker *
worker_count`: the four-worker cases process 4, 400, and 4,000 events.
Criterion's untimed setup clones the shared batch handles and completion
senders. The measured routine sends the prepared batches through the selected
routers and workers, visits every event, and waits for receiver closure. The
direct baseline measures event iteration without channels.

| Topology | Events/worker | Total events | Batches | Total time | ns/event | Events/s |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Direct, no channels | 1 | 1 | 1 | 5.398 ns | 5.398 | 185.24 million |
| Direct, no channels | 100 | 100 | 1 | 42.709 ns | 0.427 | 2.341 billion |
| Direct, no channels | 1,000 | 1,000 | 1 | 412.77 ns | 0.413 | 2.423 billion |
| 1 router, 1 worker | 1 | 1 | 1 | 12.401 us | 12,401.0 | 80.639 thousand |
| 1 router, 1 worker | 100 | 100 | 1 | 11.885 us | 118.850 | 8.414 million |
| 1 router, 1 worker | 1,000 | 1,000 | 1 | 17.406 us | 17.406 | 57.453 million |
| 2 routers, 4 workers | 1 | 4 | 4 | 40.757 us | 10,189.3 | 98.143 thousand |
| 2 routers, 4 workers | 100 | 400 | 4 | 40.871 us | 102.178 | 9.787 million |
| 2 routers, 4 workers | 1,000 | 4,000 | 4 | 41.927 us | 10.482 | 95.404 million |

The nearly flat times within each topology show that these small workloads
measure completion latency more than sustained throughput. One router and one
worker require roughly 12-17 us to cross both queues, process the batch, and
observe receiver disconnection. Two routers and four workers require roughly
41 us for four batches to traverse the
topology before the last completion-sender clone is dropped. Event work begins
to dominate only as each worker's batch grows. These measurements should be
read alongside the sustained 1,000-input benchmark above rather than used as
its replacement.

### Worker Scaling

The scaling workload limits every batch to at most 1,000 events. Larger
per-worker totals are produced by sending more batches rather than by creating
unrealistically large batches:

| Batches/worker | Events/batch | Events/worker | Total events, 4 workers | 1 worker events/s | 4 workers events/s | Speedup | Four-way efficiency |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 10 | 1,000 | 10,000 | 40,000 | 417.07 million | 814.08 million | 1.95x | 48.8% |
| 100 | 1,000 | 100,000 | 400,000 | 1.683 billion | 4.144 billion | 2.46x | 61.6% |
| 1,000 | 1,000 | 1,000,000 | 4,000,000 | 2.381 billion | 6.651 billion | 2.79x | 69.8% |

The final row therefore sends 1,000 batches through each worker: 1,000 total
batches in the one-worker topology and 4,000 total batches in the four-worker
topology. No benchmark batch contains one million events.

For the same 1,000-by-1,000 workload, direct single-thread iteration reaches
2.589 billion events/s. That gives an ideal four-thread ceiling of about 10.35
billion events/s. The complete two-router/four-worker topology reaches 6.651
billion events/s, or about 64% of that direct ceiling. Relative to the complete
one-worker topology, it scales by 2.79x. The remaining gap includes the cost of
sending and receiving 4,000 batches, queue synchronization, completion
observation, and thread scheduling and contention.

## Optional counter comparison

`benches/counters.rs` compares the same native workload in separate builds with
and without `counters`. Each trial submits 32,768 sorted events across 64 histories
using one router and either one or four workers, in submissions of one or 1,024
events. The tiny snapshot sums varying event payloads and records event count.
Timing includes public submission and waiting for completed live replay. Fresh
runtime startup, target setup, input preparation, state/status validation, and
shutdown are excluded. No query or retention application runs inside timing.
Counter-enabled trials verify exact inserted and replay totals after the drain;
both builds verify snapshot counts and sums outside timing. This is controlled
local throughput, including transport and idle-observation costs, not a production
capacity estimate or a guarantee of zero counter overhead.

```sh
cargo bench -p contime-runtime --bench counters -- --test
cargo bench -p contime-runtime --bench counters --features counters -- --test
cargo bench -p contime-runtime --bench counters -- --save-baseline counters_off
cargo bench -p contime-runtime --bench counters --features counters -- --baseline counters_off
```

Two additional one-worker, single-event-submission cases exercise bounded late
arrival (each window of 256 source events arrives in reverse order) and 50%
duplicates (each unique event is submitted twice consecutively). Their benchmark
group is `native_counter_source_submissions`; throughput counts all 32,768 source
submissions, including duplicate attempts. The duplicate case admits 16,384 unique
event applications. Both validate final states and accepted counts outside timing.
Late replay can exceed accepted events because it rebuilds prefixes; its exact
amount depends on progress between arrivals. Duplicate replay equals unique
accepted events. These supplement the four ordered cases.

Run measured builds sequentially with no other benchmark running; repeat in the
opposite order to expose changes in current machine load.
