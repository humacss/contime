# contime-runtime

The managed ConTime runtime. `ConTime::start` (also available as `Runtime::start`)
starts admission, router, and worker threads. The root `contime` crate re-exports
`ConTime`, its configuration, and operations for existing callers.

Implement Core's `Input` and snapshot/application contracts, configure worker
counts, checkpoint spacing, and history retention, then retain the returned
instance until shutdown. Inputs and timestamps must be `Send + Sync`; snapshots
and application wrappers must be movable to their worker threads. The blanket
`ThreadInput` contract applies those requirements without changing Core's
thread-local contracts.

Generic helpers accepting a managed instance use `I: ThreadInput`; Core inputs
may remain thread-local. `ThreadTopology` provides low-level channel and thread
orchestration independently of the managed `ConTime` interface.

## Responsibilities

Runtime owns the public managed instance, native message protocol, response
channels, admission coordinator, progress fences, safe retention coordination,
and the adapters connecting routers/workers to background threads. The batcher
owns packetization; Runtime integrates it after admission and before routing.
Independent request completions remain independent of transport packet sizes.
Only already-queued submissions are gathered, without waiting for more arrivals.
Gathering is bounded and stops at control messages.

History and checkpoint application use Core's synchronous storage adapter. A
small native wrapper records route coverage for in-flight admission observations
and removes that metadata before retaining events. Worker scheduling, routing,
event ordering, and snapshot algorithms remain in their specialized crates.

Submission acknowledges queueing, insertion responses acknowledge insertion,
and advancement acknowledges acceptance of a processing target. None establishes
completed live replay. Use operation-specific responses or progress in running
applications. Global idle helpers are only for controlled tests and benchmarks.

`shutdown` closes admission and joins the owned topology. It preserves router
and worker outcomes in `ShutdownReport`. ConTime does not restart failed threads
or provide persistence.

The optional `counters` feature enables cumulative worker counters and native
counted replay through Core. Replay counts include repeated work after late input
and raw events skipped by application logic; queries and retention forwarding
are excluded. Enabling counters does not change worker scheduling.

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

## Low-level thread topology

`ThreadTopology`, with `Router` and `Worker`, is the existing generic channel
and thread implementation used by process adapters and isolated diagnostics.
Its implementation lives in the private `topology` module. The descriptive
name distinguishes it from the complete `Runtime` instance.

Routers compete for one shared unbounded input queue. Each worker has a private
queue receiving from every router. The topology treats messages as opaque:
it does not coordinate admission or guarantee dispatch completion order across
routers. Runtime's higher-level protocol supplies the required fences.

`ThreadTopology::start` validates nonempty process collections and starts workers
before routers. Startup failure closes and joins the partial topology. Explicit
shutdown closes the owned sender and joins routers before workers. Drop external
sender clones before joining; process implementations must return when their
queues close. Dropping a topology without shutdown detaches its join handles.
Queue-emptiness checks alone do not establish idle or completed processing.

## Verification

```sh
cargo +1.90.0 test -p contime-runtime -p contime
cargo +1.90.0 clippy -p contime-runtime --all-targets -- -D warnings
cargo +1.90.0 bench -p contime-runtime --bench runtime
cargo +1.90.0 bench -p contime-runtime --bench breakdown
cargo +1.90.0 bench -p contime-runtime --bench batching
```

Runtime tests retain the admission/frontier, route coverage, packet completion,
process adapter, startup failure and shutdown regressions. Root integration tests
continue exercising the public facade and threaded processing end to end.

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
