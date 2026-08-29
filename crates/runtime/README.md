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

| Warm path | 1,000 batches | ns/batch | Batches/s | Logical events/s at 1,000/batch |
| --- | ---: | ---: | ---: | ---: |
| `Runtime::send` | 98.432 us | 98.432 | 10.16 million | 10.16 billion |
| Direct input sender | 99.171 us | 99.171 | 10.08 million | 10.08 billion |

The runtime is started once before timing and remains hot for the complete
Criterion run. Every timed iteration sends 1,000 opaque apply batches through
two shared-input routers and four private-input workers, then waits until all
1,000 batches have reached a worker. Startup and shutdown are outside the
timed region.

Each opaque benchmark batch represents 1,000 logical events. The runtime never
opens the batch, so logical-event throughput is the batch throughput multiplied
by 1,000 rather than a claim about event processing.

The direct case calls the same runtime-owned Crossbeam sender without the
`Runtime::send` wrapper. The wrapper measured about 0.75% faster in this run,
which is noise rather than a real speedup. The comparison therefore finds no
measurable steady-state orchestration overhead in `Runtime::send`.

Both cases include channel traffic through the hot router and worker threads,
the benchmark router's worker selection, and one atomic completion update per
batch. They exclude real snapshot routing, event storage, replay, lane
projection and filtering, checkpoint work, API rejection collection, and
every sibling ConTime crate.
