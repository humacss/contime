# contime-runtime

`contime-runtime` is the isolated process-local execution topology for
ConTime's apply path. It starts router and worker threads, connects them with
Crossbeam channels, exposes one opaque input boundary per router, and joins the complete
topology during shutdown.

The crate does not depend on `contime` or any other ConTime subcrate. It
declares local `Router` and `Worker` traits, and an eventual root orchestrator
will adapt concrete API, router, worker, event, checkpoint, and lane
implementations to those contracts.

## Topology

- Every router has one private input receiver.
- The caller selects a router by indexing the runtime's stable sender slice.
- Every worker has one private input receiver.
- Every router receives senders for the complete worker set.
- The runtime treats router input and worker input as opaque generic values.
- Router and worker factories receive stable zero-based indexes.
- Router and worker counts must both be nonzero.

The runtime owns thread and channel lifecycle only. It does not inspect,
clone, route, apply, replay, or otherwise interpret messages.

## Lifecycle

`Runtime::start` validates `RuntimeConfig`, creates the channels, starts all
workers, and then starts all routers. Threads receive deterministic names:
`contime-worker-{index}` and `contime-router-{index}`.

If a thread cannot be spawned, startup closes the partial topology and joins
every thread already started before returning `StartError::ThreadSpawn`.

A running `Runtime` offers `send(router_index, input)` and borrowed access to
its router input senders. The runtime does not inspect an input to choose its
router. Callers may clone senders, but explicit shutdown cannot finish until
all external clones have been dropped.

`Runtime::shutdown` drops the runtime-owned input sender, joins every router,
then joins every worker after the router-owned worker senders have closed. Its
report preserves one ordered outcome per thread and distinguishes completion,
a returned implementation error, and a panic. The runtime observes failures
but does not restart or recover failed threads in this version.

Queries, advanced/time operations, memory policy, routing policy, replay,
snapshots, automatic restarts, and live health monitoring are outside this
crate.

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

Local release-mode Criterion results recorded on 2026-08-29:

| Topology | 1,000 inputs | ns/input | Inputs/s |
| --- | ---: | ---: | ---: |
| 1 router, 1 worker | 49.761 us | 49.761 | 20.096 million |
| 2 routers, 4 workers | 65.612 us | 65.612 | 15.241 million |

The runtime is started once before timing and remains hot for the complete
Criterion run. Every timed iteration sends 1,000 benchmark events through the
router sender selected by `event.router_index`. The selected router forwards
the event through the worker sender selected by `event.worker_index`. There is
no hash, modulo, runtime trait lookup, or runtime message inspection in the
timed path. Criterion's untimed setup creates the completion channel and
clones its sender into every prepared input. The measured routine only sends
those inputs and waits for the receiver to disconnect after workers process
them and drop every sender clone. No acknowledgement messages are sent.

Both cases include two Crossbeam channel hops, the sender-slice index accesses,
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
