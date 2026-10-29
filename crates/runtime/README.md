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

The optional `monitor` feature enables cumulative worker counters and native
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

## Benchmark Snapshot

> These measurements predate the 2026-08-31 shared-router-queue API change.
> They are retained as historical results. The full benchmark run below records
> measurements of the current implementation.

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
cargo bench -p contime-runtime --bench counters --features monitor -- --test
cargo bench -p contime-runtime --bench counters -- --save-baseline counters_off
cargo bench -p contime-runtime --bench counters --features monitor -- --baseline counters_off
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

## Full benchmark run — 2026-10-28

Measured on Apple M3 Pro (11 logical CPUs), Rust 1.90.0, optimized Cargo bench
profile, with unpinned threads and normal desktop background activity. All groups
ran sequentially after compilation. Existing warm-up, sample-count and measurement
settings were retained; no shortened timing overrides were used. These are local
measurements, not production-capacity guarantees. The workspace lockfile uses
Criterion 0.5.1, Crossbeam channel 0.5.15 and Crossbeam utils 0.8.21.
Earlier dated results above
remain historical records and may use different fixtures or settings.

Tables report the **median time per benchmark iteration**, with its bootstrap
95% confidence interval. Rates are derived from that median only where the
benchmark declares a throughput unit; that unit may be source events, target
applications, snapshots, bytes, or bookkeeping calls. It is not automatically
ConTime's event-processing capacity. Thread-startup and allocation boundaries
remain those defined in each benchmark source.

Feature variants were measured in separate sequential runs, not interleaved
paired trials; changing background load can affect their differences.

Historical 0.1.0 feature sets (before monitoring enabled counting automatically):
`all-features` is the workspace feature union, including optional
worker counters; `default` disables optional counters; `status-no-counters`
enables Monitor's `runtime` feature without counters. Each table states the
actual sample count. Inline tests may contain multiple Criterion cases.

The `runtime`, `batching`, and `breakdown` targets use synthetic handlers around
`ThreadTopology`. They exclude managed admission, event-history storage,
checkpointing and replay. The `counters` target exercises the full managed
`ConTime` pipeline with real storage and live replay. Their rates describe
different operations and must not be treated as interchangeable.

### `batching` · criterion · all-features

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `batching/realistic_batches/1_routers_1_workers/1000_batches_per_worker_1000_events_per_batch_1000000_events_per_worker` | 341.383 µs | 340.863 µs–342.001 µs | 50 | 2.929 G elements/s |
| `batching/realistic_batches/1_routers_1_workers/100_batches_per_worker_1000_events_per_batch_100000_events_per_worker` | 48.322 µs | 48.160 µs–48.450 µs | 50 | 2.069 G elements/s |
| `batching/realistic_batches/1_routers_1_workers/10_batches_per_worker_1000_events_per_batch_10000_events_per_worker` | 13.301 µs | 13.079 µs–13.503 µs | 50 | 751.796 M elements/s |
| `batching/realistic_batches/1_routers_1_workers/1_batches_per_worker_1000_events_per_batch_1000_events_per_worker` | 2.903 µs | 2.762 µs–3.041 µs | 50 | 344.476 M elements/s |
| `batching/realistic_batches/1_routers_1_workers/1_batches_per_worker_100_events_per_batch_100_events_per_worker` | 2.616 µs | 2.560 µs–2.723 µs | 50 | 38.229 M elements/s |
| `batching/realistic_batches/1_routers_1_workers/1_batches_per_worker_1_events_per_batch_1_events_per_worker` | 2.863 µs | 2.793 µs–3.108 µs | 50 | 349.286 k elements/s |
| `batching/realistic_batches/2_routers_4_workers/1000_batches_per_worker_1000_events_per_batch_1000000_events_per_worker` | 561.413 µs | 555.960 µs–574.522 µs | 50 | 7.125 G elements/s |
| `batching/realistic_batches/2_routers_4_workers/100_batches_per_worker_1000_events_per_batch_100000_events_per_worker` | 79.823 µs | 77.927 µs–82.730 µs | 50 | 5.011 G elements/s |
| `batching/realistic_batches/2_routers_4_workers/10_batches_per_worker_1000_events_per_batch_10000_events_per_worker` | 39.392 µs | 39.205 µs–39.726 µs | 50 | 1.015 G elements/s |
| `batching/realistic_batches/2_routers_4_workers/1_batches_per_worker_1000_events_per_batch_1000_events_per_worker` | 28.792 µs | 28.250 µs–29.512 µs | 50 | 138.927 M elements/s |
| `batching/realistic_batches/2_routers_4_workers/1_batches_per_worker_100_events_per_batch_100_events_per_worker` | 26.594 µs | 26.465 µs–27.361 µs | 50 | 15.041 M elements/s |
| `batching/realistic_batches/2_routers_4_workers/1_batches_per_worker_1_events_per_batch_1_events_per_worker` | 26.630 µs | 25.817 µs–26.954 µs | 50 | 150.207 k elements/s |
| `batching/realistic_batches/direct_no_channels/1000_batches_1000_events_each` | 308.712 µs | 308.159 µs–312.162 µs | 50 | 3.239 G elements/s |
| `batching/realistic_batches/direct_no_channels/100_batches_1000_events_each` | 30.908 µs | 30.800 µs–31.145 µs | 50 | 3.235 G elements/s |
| `batching/realistic_batches/direct_no_channels/10_batches_1000_events_each` | 3.082 µs | 3.059 µs–3.124 µs | 50 | 3.245 G elements/s |
| `batching/realistic_batches/direct_no_channels/1_batches_1000_events_each` | 319.678 ns | 315.567 ns–329.340 ns | 50 | 3.128 G elements/s |
| `batching/realistic_batches/direct_no_channels/1_batches_100_events_each` | 30.778 ns | 30.741 ns–30.866 ns | 50 | 3.249 G elements/s |
| `batching/realistic_batches/direct_no_channels/1_batches_1_events_each` | 0.802 ns | 0.801 ns–0.803 ns | 50 | 1.247 G elements/s |

### `batching` · criterion · default

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `batching/realistic_batches/1_routers_1_workers/1000_batches_per_worker_1000_events_per_batch_1000000_events_per_worker` | 345.555 µs | 343.365 µs–350.078 µs | 50 | 2.894 G elements/s |
| `batching/realistic_batches/1_routers_1_workers/100_batches_per_worker_1000_events_per_batch_100000_events_per_worker` | 47.255 µs | 47.140 µs–47.604 µs | 50 | 2.116 G elements/s |
| `batching/realistic_batches/1_routers_1_workers/10_batches_per_worker_1000_events_per_batch_10000_events_per_worker` | 13.846 µs | 13.149 µs–14.752 µs | 50 | 722.256 M elements/s |
| `batching/realistic_batches/1_routers_1_workers/1_batches_per_worker_1000_events_per_batch_1000_events_per_worker` | 3.032 µs | 2.551 µs–4.062 µs | 50 | 329.787 M elements/s |
| `batching/realistic_batches/1_routers_1_workers/1_batches_per_worker_100_events_per_batch_100_events_per_worker` | 2.993 µs | 2.820 µs–3.235 µs | 50 | 33.415 M elements/s |
| `batching/realistic_batches/1_routers_1_workers/1_batches_per_worker_1_events_per_batch_1_events_per_worker` | 2.910 µs | 2.732 µs–3.177 µs | 50 | 343.664 k elements/s |
| `batching/realistic_batches/2_routers_4_workers/1000_batches_per_worker_1000_events_per_batch_1000000_events_per_worker` | 552.741 µs | 549.131 µs–560.357 µs | 50 | 7.237 G elements/s |
| `batching/realistic_batches/2_routers_4_workers/100_batches_per_worker_1000_events_per_batch_100000_events_per_worker` | 78.211 µs | 77.983 µs–78.455 µs | 50 | 5.114 G elements/s |
| `batching/realistic_batches/2_routers_4_workers/10_batches_per_worker_1000_events_per_batch_10000_events_per_worker` | 40.004 µs | 39.377 µs–41.242 µs | 50 | 999.902 M elements/s |
| `batching/realistic_batches/2_routers_4_workers/1_batches_per_worker_1000_events_per_batch_1000_events_per_worker` | 29.949 µs | 28.970 µs–31.508 µs | 50 | 133.561 M elements/s |
| `batching/realistic_batches/2_routers_4_workers/1_batches_per_worker_100_events_per_batch_100_events_per_worker` | 26.713 µs | 25.892 µs–27.669 µs | 50 | 14.974 M elements/s |
| `batching/realistic_batches/2_routers_4_workers/1_batches_per_worker_1_events_per_batch_1_events_per_worker` | 27.284 µs | 26.491 µs–27.784 µs | 50 | 146.607 k elements/s |
| `batching/realistic_batches/direct_no_channels/1000_batches_1000_events_each` | 307.411 µs | 305.554 µs–310.732 µs | 50 | 3.253 G elements/s |
| `batching/realistic_batches/direct_no_channels/100_batches_1000_events_each` | 30.799 µs | 30.615 µs–31.458 µs | 50 | 3.247 G elements/s |
| `batching/realistic_batches/direct_no_channels/10_batches_1000_events_each` | 3.051 µs | 3.044 µs–3.070 µs | 50 | 3.277 G elements/s |
| `batching/realistic_batches/direct_no_channels/1_batches_1000_events_each` | 331.894 ns | 316.654 ns–355.382 ns | 50 | 3.013 G elements/s |
| `batching/realistic_batches/direct_no_channels/1_batches_100_events_each` | 31.028 ns | 30.830 ns–31.343 ns | 50 | 3.223 G elements/s |
| `batching/realistic_batches/direct_no_channels/1_batches_1_events_each` | 0.805 ns | 0.803 ns–0.806 ns | 50 | 1.243 G elements/s |

### `breakdown` · criterion · all-features

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `breakdown/bounded_one_thread_hop/1000` | 20.506 µs | 20.442 µs–20.864 µs | 30 | 48.767 M elements/s |
| `breakdown/bounded_one_thread_hop/10000` | 174.013 µs | 169.710 µs–179.825 µs | 30 | 57.467 M elements/s |
| `breakdown/bounded_router_and_worker_hops/1000` | 42.931 µs | 41.776 µs–45.045 µs | 30 | 23.293 M elements/s |
| `breakdown/bounded_router_and_worker_hops/10000` | 446.284 µs | 443.704 µs–449.645 µs | 30 | 22.407 M elements/s |
| `breakdown/local_send_and_recv/1` | 99.393 ns | 98.355 ns–100.726 ns | 30 | 10.061 M elements/s |
| `breakdown/local_send_and_recv/10` | 228.318 ns | 225.129 ns–234.985 ns | 30 | 43.799 M elements/s |
| `breakdown/local_send_and_recv/100` | 1.463 µs | 1.449 µs–1.471 µs | 30 | 68.375 M elements/s |
| `breakdown/local_send_and_recv/1000` | 13.774 µs | 13.671 µs–13.973 µs | 30 | 72.598 M elements/s |
| `breakdown/local_send_and_recv/10000` | 141.142 µs | 140.644 µs–142.040 µs | 30 | 70.850 M elements/s |
| `breakdown/one_thread_hop/1` | 396.804 ns | 391.772 ns–404.934 ns | 30 | 2.520 M elements/s |
| `breakdown/one_thread_hop/10` | 669.354 ns | 661.250 ns–691.567 ns | 30 | 14.940 M elements/s |
| `breakdown/one_thread_hop/100` | 2.677 µs | 2.668 µs–2.711 µs | 30 | 37.350 M elements/s |
| `breakdown/one_thread_hop/1000` | 19.441 µs | 19.322 µs–19.926 µs | 30 | 51.438 M elements/s |
| `breakdown/one_thread_hop/10000` | 116.649 µs | 111.413 µs–125.222 µs | 30 | 85.728 M elements/s |
| `breakdown/router_and_worker_hops/1` | 3.155 µs | 2.867 µs–3.436 µs | 30 | 316.930 k elements/s |
| `breakdown/router_and_worker_hops/10` | 5.156 µs | 4.867 µs–5.329 µs | 30 | 1.939 M elements/s |
| `breakdown/router_and_worker_hops/100` | 9.443 µs | 9.375 µs–9.481 µs | 30 | 10.590 M elements/s |
| `breakdown/router_and_worker_hops/1000` | 46.068 µs | 45.274 µs–46.794 µs | 30 | 21.707 M elements/s |
| `breakdown/router_and_worker_hops/10000` | 385.966 µs | 375.908 µs–391.815 µs | 30 | 25.909 M elements/s |

### `breakdown` · criterion · default

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `breakdown/bounded_one_thread_hop/1000` | 20.079 µs | 19.994 µs–20.234 µs | 30 | 49.802 M elements/s |
| `breakdown/bounded_one_thread_hop/10000` | 177.118 µs | 173.169 µs–179.167 µs | 30 | 56.460 M elements/s |
| `breakdown/bounded_router_and_worker_hops/1000` | 41.128 µs | 40.937 µs–41.238 µs | 30 | 24.314 M elements/s |
| `breakdown/bounded_router_and_worker_hops/10000` | 452.515 µs | 449.465 µs–457.723 µs | 30 | 22.099 M elements/s |
| `breakdown/local_send_and_recv/1` | 96.893 ns | 96.474 ns–98.878 ns | 30 | 10.321 M elements/s |
| `breakdown/local_send_and_recv/10` | 242.304 ns | 238.864 ns–285.259 ns | 30 | 41.271 M elements/s |
| `breakdown/local_send_and_recv/100` | 1.580 µs | 1.476 µs–1.687 µs | 30 | 63.287 M elements/s |
| `breakdown/local_send_and_recv/1000` | 13.635 µs | 13.607 µs–13.671 µs | 30 | 73.342 M elements/s |
| `breakdown/local_send_and_recv/10000` | 141.242 µs | 140.950 µs–141.893 µs | 30 | 70.801 M elements/s |
| `breakdown/one_thread_hop/1` | 395.767 ns | 388.202 ns–481.334 ns | 30 | 2.527 M elements/s |
| `breakdown/one_thread_hop/10` | 674.855 ns | 670.714 ns–693.390 ns | 30 | 14.818 M elements/s |
| `breakdown/one_thread_hop/100` | 2.679 µs | 2.667 µs–2.711 µs | 30 | 37.323 M elements/s |
| `breakdown/one_thread_hop/1000` | 19.390 µs | 19.271 µs–19.448 µs | 30 | 51.573 M elements/s |
| `breakdown/one_thread_hop/10000` | 123.171 µs | 109.042 µs–130.145 µs | 30 | 81.188 M elements/s |
| `breakdown/router_and_worker_hops/1` | 4.265 µs | 3.819 µs–4.398 µs | 30 | 234.493 k elements/s |
| `breakdown/router_and_worker_hops/10` | 4.903 µs | 4.678 µs–5.549 µs | 30 | 2.040 M elements/s |
| `breakdown/router_and_worker_hops/100` | 9.689 µs | 9.605 µs–9.801 µs | 30 | 10.322 M elements/s |
| `breakdown/router_and_worker_hops/1000` | 46.195 µs | 46.027 µs–46.570 µs | 30 | 21.648 M elements/s |
| `breakdown/router_and_worker_hops/10000` | 386.081 µs | 380.576 µs–393.327 µs | 30 | 25.901 M elements/s |

### `counters` · criterion · all-features

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `native_counter_source_submissions/duplicates_50pct/1_worker_batch_1` | 5.432 ms | 5.322 ms–6.157 ms | 20 | 6.033 M elements/s |
| `native_counter_source_submissions/late_within_256/1_worker_batch_1` | 13.717 ms | 12.594 ms–15.248 ms | 20 | 2.389 M elements/s |
| `native_counters/1_workers/batch_1` | 8.672 ms | 8.653 ms–8.733 ms | 20 | 3.778 M elements/s |
| `native_counters/1_workers/batch_1024` | 8.598 ms | 8.550 ms–8.662 ms | 20 | 3.811 M elements/s |
| `native_counters/4_workers/batch_1` | 3.860 ms | 3.847 ms–3.882 ms | 20 | 8.489 M elements/s |
| `native_counters/4_workers/batch_1024` | 2.131 ms | 2.120 ms–2.167 ms | 20 | 15.375 M elements/s |

### `counters` · criterion · default

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `native_counter_source_submissions/duplicates_50pct/1_worker_batch_1` | 5.625 ms | 5.559 ms–5.907 ms | 20 | 5.825 M elements/s |
| `native_counter_source_submissions/late_within_256/1_worker_batch_1` | 12.692 ms | 12.093 ms–16.091 ms | 20 | 2.582 M elements/s |
| `native_counters/1_workers/batch_1` | 9.111 ms | 9.085 ms–9.382 ms | 20 | 3.597 M elements/s |
| `native_counters/1_workers/batch_1024` | 8.997 ms | 8.971 ms–9.002 ms | 20 | 3.642 M elements/s |
| `native_counters/4_workers/batch_1` | 3.844 ms | 3.832 ms–3.868 ms | 20 | 8.525 M elements/s |
| `native_counters/4_workers/batch_1024` | 2.259 ms | 2.238 ms–2.299 ms | 20 | 14.505 M elements/s |

### `runtime` · criterion · all-features

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `runtime/1000_inputs/topology/1_routers_1_workers` | 37.818 µs | 37.591 µs–38.382 µs | 50 | 26.442 M elements/s |
| `runtime/1000_inputs/topology/2_routers_4_workers` | 93.602 µs | 93.242 µs–93.749 µs | 50 | 10.683 M elements/s |

### `runtime` · criterion · default

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `runtime/1000_inputs/topology/1_routers_1_workers` | 37.232 µs | 37.113 µs–37.317 µs | 50 | 26.859 M elements/s |
| `runtime/1000_inputs/topology/2_routers_4_workers` | 98.497 µs | 96.870 µs–102.480 µs | 50 | 10.153 M elements/s |

### `contime_runtime` · inline · all-features

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `core/listen/replayed_notification` | 31.209 ns | 31.161 ns–31.494 ns | 100 | — |
| `core/message/1000_routes` | 486.834 ns | 481.413 ns–495.564 ns | 100 | — |
| `core/router/1000_routes_one_worker` | 4.435 µs | 4.416 µs–4.461 µs | 100 | — |
| `core/send/prepare_and_forward_1000` | 10.760 µs | 10.725 µs–10.856 µs | 100 | — |
| `core/shutdown/one_router_one_worker` | 34.475 µs | 34.315 µs–34.600 µs | 100 | — |
| `core/start/one_router_one_worker` | 14.956 µs | 14.677 µs–15.244 µs | 100 | — |
| `core/worker/1000_events_one_snapshot` | 88.369 µs | 88.129 µs–89.248 µs | 100 | — |
