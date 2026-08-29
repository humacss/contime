# contime-runtime

`contime-runtime` is the isolated process-local execution topology for
ConTime's apply path. It starts router and worker threads, connects them with
Crossbeam channels, exposes one opaque input boundary, and joins the complete
topology during shutdown.

The crate does not depend on `contime` or any other ConTime subcrate. It
declares local `Router` and `Worker` traits, and an eventual root orchestrator
will adapt concrete API, router, worker, event, checkpoint, and lane
implementations to those contracts.

## Topology

- All router threads compete on one shared input receiver. The first available
  router consumes the next complete message.
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

A running `Runtime` offers `send` and borrowed access to its input sender.
Callers may clone that sender, but explicit shutdown cannot finish until all
external clones have been dropped.

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

Run the isolated lifecycle benchmark:

```bash
cargo test --release --manifest-path crates/runtime/Cargo.toml \
  runtime::tests::benchmark_runtime -- --ignored --nocapture --test-threads=1
```

## Benchmark Snapshot

Local release-mode Criterion results recorded on 2026-08-29:

| Workload | Median | ns/input | Inputs/s |
| --- | ---: | ---: | ---: |
| 1,000 inputs, 2 routers, 4 workers | 155.40 us | 155.40 | 6.44 million |

Each timed iteration starts a new runtime, starts two router threads and four
worker threads, sends 1,000 `u64` inputs through the public runtime boundary,
routes each input to one worker, confirms all 1,000 inputs were consumed,
shuts down the topology, and joins all six threads. Prepared input vectors and
the shared consumption counter are created outside the timed routine.

The result measures complete isolated lifecycle overhead rather than only
steady-state channel throughput. It excludes real snapshot routing, event
storage, replay, lane projection and filtering, checkpoint work, API rejection
collection, and every sibling ConTime crate.
