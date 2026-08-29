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
| 1 router, 1 worker | 67.104 us | 67.104 | 14.902 million |
| 2 routers, 4 workers | 89.763 us | 89.763 | 11.140 million |

The runtime is started once before timing and remains hot for the complete
Criterion run. Every timed iteration sends 1,000 benchmark events through the
router sender selected by `event.router_index`. The selected router forwards
the event through the worker sender selected by `event.worker_index`. There is
no hash, modulo, runtime trait lookup, or runtime message inspection in the
timed path. A constant number of flush messages confirms that all router and
worker queues drained; there is no per-event acknowledgement or atomic.

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

The batching benchmark holds total work constant at 100,000 logical events.
Prepared batch payloads are shared slices. Timed submission clones one shared
batch handle, sends the batch through the selected router and worker, and the
worker visits every event. The direct baseline performs the same batch-handle
clone and event iteration without channels.

| Topology | Events/batch | Channel messages | Total time | ns/event | Events/s |
| --- | ---: | ---: | ---: | ---: | ---: |
| Direct, no channels | 1 | 100,000 | 241.35 us | 2.414 | 414.34 million |
| Direct, no channels | 100 | 1,000 | 38.647 us | 0.386 | 2.588 billion |
| Direct, no channels | 1,000 | 100 | 38.646 us | 0.386 | 2.588 billion |
| 1 router, 1 worker | 1 | 100,000 | 2.4931 ms | 24.931 | 40.111 million |
| 1 router, 1 worker | 100 | 1,000 | 74.870 us | 0.749 | 1.336 billion |
| 1 router, 1 worker | 1,000 | 100 | 61.702 us | 0.617 | 1.621 billion |
| 2 routers, 4 workers | 1 | 100,000 | 2.1134 ms | 21.134 | 47.318 million |
| 2 routers, 4 workers | 100 | 1,000 | 98.774 us | 0.988 | 1.012 billion |
| 2 routers, 4 workers | 1,000 | 100 | 77.381 us | 0.774 | 1.292 billion |

For one router and one worker, subtracting the direct baseline leaves about
2.252 ms of channel overhead for single-event messages, 36.223 us for batches
of 100, and 23.056 us for batches of 1,000. Batching 100 events therefore
reduces total latency by 33.3 times and batching 1,000 reduces it by 40.4
times. At the larger sizes, the fixed 38.65 us event-iteration baseline is the
majority of the remaining measurement, so increasing the batch beyond 100 has
diminishing returns for this no-op workload.
