# contime-monitor

Worker-status collection and passive workload accounting, with presentation kept
separate. The default build is standard-library-only. Enable `workers` for the
nonblocking collector and protocol, or `runtime` for the managed ConTime adapter.
The collector starts no threads, sleeps, or application queries. Rust 1.90 or newer
is required.

## Worker status data layer

`WorkerMonitor::attach(&contime)` borrows a running managed instance. Call
`begin(elapsed)` to request status, then `poll(elapsed)` from the host loop until
it returns a complete immutable `Arc<WorkerReading<_>>`. There is at most one
in-flight collection. `cancel()` drops the receiver; keep polling or cancel a
stalled request according to the host's deadline. All supplied times must be
nondecreasing. The most recent 120 complete readings are retained by default;
`with_capacity` accepts an explicit nonzero bound.

Reports contain initialized history-store and metadata-only counts, scheduled
and currently due history counts, the accepted processing target and admission
horizon, queued pruning passes, and a coordination round awaiting resolution.
These are worker-owned orchestration gauges, not event counts or processing
rates. A pending history may contain many events; a due history may still be
blocked. Accepted targets and horizons do not mean replay or pruning completed.

Status requests go directly to workers and do not establish a prefix fence over
producer/router/admission queues. Each worker reports independently between
operations; a complete collection is not an atomic global observation. No
snapshot reconstruction or snapshot-ID enumeration is performed. Snapshot
queries remain the separate way to inspect application state.

The collector validates worker IDs, duplicates, stream closure, and complete
coverage. Partial/invalid responses never replace the latest reading. Each
`poll` is nonblocking and drains at most the expected worker count plus one
message. `WorkerStatusSource<T>` allows transport-free test sources or other
host adapters. Terminal formatting/input/rendering do not belong to this data
layer; the immutable reading can be handed to a separate presenter.

```no_run
# #[cfg(feature = "workers")]
# fn collect_once(contime: &impl contime_monitor::WorkerStatusSource<u64>) -> Result<(), contime_monitor::WorkerMonitorError> {
// Enable the `runtime` feature to attach directly to a managed ConTime instance.
let clock = std::time::Instant::now();
let mut monitor = contime_monitor::WorkerMonitor::attach(contime);
monitor.begin(clock.elapsed())?;
// On subsequent host-loop turns, poll until a complete reading is available:
if let Some(reading) = monitor.poll(clock.elapsed())? {
    for worker in &reading.workers {
        println!("worker {}: {} history stores", worker.worker_id, worker.history_stores);
    }
}
# Ok(())
# }
```

## Optional application counters and rates

Enable `counters` alongside `runtime` to instrument the managed native workers.
At the root facade, use `features = ["monitor", "counters"]`. Cargo features are
additive across a build; keep comparison builds separate. Counters are disabled
by default and are optional independently of presentation.

`WorkerStatus::counters` is `None` when instrumentation is disabled. Available
`WorkerCounters` contain cumulative `inserted_applications`, optional
`replay_applications`, and a saturation flag. Insertions count accepted changes
to individual histories, excluding duplicates and rejections. Live replay counts
raw canonical events traversed, including unchanged prefixes replayed after late
input and events ignored by application logic. Snapshot queries and retention
reconstruction do not contribute. Fanout counts once per target history, so
these totals are **applications, not unique source events**. Subtracting replay
from insertions does not produce a valid outstanding-work count.

Workers accumulate insertion totals once per batch and replay totals once per
processing step. Native storage derives replay amounts from existing checkpoint
counts; there are no added per-event atomics, locks, allocations, or clock reads.
Custom worker stores may implement `process_until_counted`; the default runs
ordinary processing and reports an unknown amount. After any unknown operation,
that worker's cumulative replay count stays unavailable, rather than pretending
it processed zero. Counter overflow saturates and marks the totals as lower bounds.

`ApplicationActivity::between(previous, current)` derives per-worker and aggregate
counts/rates independently of collection and presentation. Readings must come
from the same runtime instance; start a fresh view/baseline when changing sources.
The first reading,
zero/backwards elapsed time, a reset, missing data, or saturation produces
unavailable rates. Worker membership/order changes invalidate aggregate rates.
Valid idle intervals produce a real zero. Rates use host completion-clock
intervals, not application timestamps; collection delay and independent worker
capture times introduce sampling uncertainty. Presentation code can format these values as needed; unavailable rates remain
explicit `None` values.

For isolated feature-cost measurements, run the same native workload in separate
builds, then reverse their order. It prepares fixed cheap events, validates live
completion and exact counts, and excludes startup/state verification/shutdown:

```sh
cargo bench -p contime-runtime --bench counters
cargo bench -p contime-runtime --features counters --bench counters
cargo bench -p contime-monitor --features workers --bench activity
```

## Presenting monitor data

The monitor has no UI or terminal dependencies. Use `WorkerReading::workers` for
worker gauges and counters, and `ApplicationActivity::between(previous, current)`
for per-worker and aggregate counts and rates. Keep the first reading as a
baseline for cumulative average rates, or use the previous reading for interval
rates. Unavailable values remain explicit; do not turn them into zero.

Collection, rendering, refresh cadence, clocks and input belong to the consumer.
The [live example guide](../../examples/README.md) shows how the cases share a
terminal dashboard built from these public APIs. Its renderer, UI tests and
rendering benchmarks live outside this crate under `examples/support/ui/`.

## Workload accounting

The following compatibility API records submitted and query-confirmed work. It
is independent of worker-status collection and does not read worker execution
progress. Simulator consumers can continue using it.

The host records source events separately from destination applications: one
event routed to three histories counts as one event and three applications.
No monitoring fields are required on consumer snapshots, and submission counts
can be recorded once per batch rather than once per event.

```rust
use contime_monitor::{Monitor, Totals};
use std::time::Duration;

let mut monitor = Monitor::default();
monitor.sample(Duration::ZERO)?;
monitor.record_submission(Totals { events: 2, applications: 6 })?;
let query = monitor.start_query(Duration::from_secs(1))?;
// Issue a host query that covers the captured prefix and relevant destinations.
monitor.record_submission(Totals { events: 1, applications: 2 })?;
// Once the host has established that query's coverage and received its result:
monitor.complete_query(query, Duration::from_secs(2))?;
let view = monitor.sample(Duration::from_secs(2))?;
assert_eq!(view.confirmed.events, 2);
assert_eq!(view.outstanding.events, 1);
# Ok::<(), contime_monitor::Error>(())
```

`start_query` captures all recorded submissions. If an ordered query gate covers
an earlier admitted prefix while newer submissions are already recorded, use
`start_query_through(at, admitted_totals)`. Both counters must lie between the
confirmed prefix and current submissions; an invalid prefix is rejected without
changing state. The host remains responsible for establishing query coverage.

A completed query confirms only its captured submission prefix. The monitor does
not issue queries or establish admission, insertion, destination coverage, or
ordering guarantees. The host must establish those guarantees before reporting
completion. Queries may reconstruct retained historical state independently of
live replay, so **query-confirmed work is not live-replay completion**. These
counts do not measure actual replay calls, duplicate handling, or execution cost.

At most one query is active. Return its entire unchanged token to completion;
stale and fabricated tokens are rejected. Tokens belong to one monitor and must
not be exchanged between monitors. All timed mutations require nondecreasing
caller-supplied `Duration` values. Read-only snapshots validate against the most
recent mutation but do not advance that clock. Errors leave all state unchanged.

`sample` stores bounded history (120 entries by default; `Monitor::new` accepts a
nonzero capacity). Recent rates compare against the previous sample; average
rates compare against the first sample, excluding the initial pre-sample counts.
Without a baseline or with zero elapsed time, rates are zero. A read-only
`snapshot` does not reset rate intervals. `older_outstanding` counts the previous
sample's submission prefix still unconfirmed, excluding newly submitted work.

Submissions have no clock argument. Their age is estimated from observation
intervals, using the previous sample boundary, or the first observation time
when no sample exists. It is not an exact enqueue timestamp. Sampling creates at
most one cohort per interval. Both cohort and sample storage are bounded by the
configured capacity. When old unconfirmed cohorts are compacted, the earliest
boundary is preserved; partially confirmed compacted cohorts can consequently
overestimate the remaining work's age. Confirmation removes fully covered
cohorts, so the age becomes absent when all work is confirmed. Sample regularly
for finer age resolution. There is no per-event log.

Verify with `cargo test -p contime-monitor` and
`cargo clippy -p contime-monitor --all-targets -- -D warnings`.

MIT licensed.

## Tests and benchmarks

Criterion is development-only. The Runtime/protocol/channel dependencies are
optional for consumers; builds without features remain standard-library-only.
Run from the workspace root:

```sh
cargo +1.90.0 test -p contime-monitor
cargo +1.90.0 clippy -p contime-monitor --all-targets -- -D warnings
cargo +1.90.0 bench -p contime-monitor --bench arithmetic
cargo +1.90.0 bench -p contime-monitor --bench operations
cargo +1.90.0 bench -p contime-monitor --bench pipeline
cargo +1.90.0 bench -p contime-monitor --bench sustained
# Repeat with instrumentation modes visited in the opposite order:
CONTIME_MONITOR_REVERSE=1 cargo +1.90.0 bench -p contime-monitor --bench pipeline
CONTIME_MONITOR_REVERSE=1 cargo +1.90.0 bench -p contime-monitor --bench sustained
```

The existing `arithmetic`, `operations`, `pipeline`, and `sustained` benchmarks
cover the compatibility workload-accounting API. They do not measure the
worker-status collector. Use `workers` below for direct worker observation.

| Unit | Correctness coverage | Benchmark |
| --- | --- | --- |
| Totals and rates | Checked addition, both overflow cases, independent saturation, fractional/zero durations | `arithmetic`: addition, overflow, subtraction, saturation, rates |
| Errors | All descriptions and failure atomicity | `arithmetic/error_display`; rejected operations |
| Construction | Empty state and no history/cohort allocation | `operations/default`, `new` |
| Submission accounting | Individual/batched totals, fanout, overflow, no observation changes | `record_submission` at 1, 64 and 4,096 source events per call |
| Query capture | Complete and explicitly admitted prefixes; active, invalid and exhausted identities | `start_query`, `start_query_through`, rejection paths |
| Query completion | Exact tokens, latency, captured prefixes and cohort removal | `complete_query` at 0, 1, 120 and 4,096 retained cohorts |
| Snapshot/rate reads | Baselines, clock validation, outstanding work and age | `snapshot` at capacities 1, 120 and 4,096 |
| Sampling and compaction | Bounded storage, rollover and oldest age preservation | First allocation, history rollover and new-cohort compaction |
| History and state accessors | Chronological/reverse iteration, clocks and query lifecycle | Full history iteration and each individual accessor |
| Runtime integration | Counts and canonical hashes, late arrivals, one/multiple workers, fanout and historical prefixes | Identical Runtime work with monitoring disabled/per event/per 64 events |
| Sustained integration | Persistent Runtime, periodic deadlines, cap/retention, cumulative state and monitor rollover | `sustained`: periodic monitoring enabled versus disabled |

Arithmetic benchmarks compile the actual private arithmetic module into the
benchmark executable. No benchmark-only public API is added. Derived data types
are covered through the operations that consume them. Private clock and cohort
helpers are covered through their public operations, including rejection and
cleanup paths.

Operations use opaque inputs and consumed outputs. Sampling runs with bounded,
pre-populated state. Query-capture/completion fixtures are rebuilt outside the
measured section; monitor destruction is excluded from those measurements.
Compaction includes one submission-recording call. History iteration includes a
checksum over all retained samples. Submission throughput describes bookkeeping
units per call, **not** ConTime event-processing capacity.

The shared [Runtime fixture](benches/support/pipeline.rs) is exercised by both
integration tests and the `pipeline` benchmark. Each trial prepares 4,096 unique
events, 128 snapshots, one or four workers, and one or three targets per event.
Reversing 256-event arrival blocks makes events late within each history.
All variants send the same individual events through the same Runtime APIs;
only monitor bookkeeping changes. Per-64-event monitoring does not batch
ConTime submissions. Tests additionally cover an incomplete recording batch.

Pipeline timing includes submission, insertion responses, settled replay, a
snapshot query and monitor bookkeeping. Controlled replay uses `wait_until_idle`
only for this test/benchmark fixture. Input/reference generation, thread startup,
state validation and shutdown are excluded. Every trial starts with fresh state,
and validates counts and an independent canonical rolling hash. Monitor metrics
use the real host elapsed clock, including validation; those metric rates and
latencies are not the Criterion timing result.

Use release measurements and repeat the pipeline comparison. Thread scheduling,
allocator behavior and other activity can overwhelm the cost of a few counter
updates. A monitored run appearing faster is not evidence that monitoring speeds
up ConTime. Results here measure local bookkeeping and a fixed diagnostic
workload; they do not predict sustained application throughput.


### Compatibility workload-accounting sustained comparison

The [`sustained`](benches/sustained.rs) benchmark keeps one Runtime alive across
warmup and all measured samples. It compares monitoring disabled against
per-event submission accounting with periodic monitor sampling. Both modes use
the same event generation, canonical reference updates, individual submission
calls, insertion fences, queries, validation and pending-work cap. There is no
simulator, Maximum controller, UI or monitor-dependent backpressure.

Cases use one or four workers, 128 snapshots, one or three targets per event,
128-event checkpoints, and reversed 256-event arrival blocks. Queries and
monitor samples have fixed 50 ms deadlines (20 Hz). Missed ticks are skipped
without shifting the schedule's phase. This is a single-caller producer: it
pauses for the periodic query's insertion fence, response and validation, while
Runtime workers continue running. Sampling deadlines are checked at block
boundaries. No sleeps are used to generate load.

At most 262,144 events await query confirmation. Reaching this common safety
bound triggers an additional query in either mode. Native retention keeps
262,144 logical event-time ticks, with a 10 ms pruning coordination interval.
The benchmark reports periodic, capacity-triggered and final query counts,
scheduled sample ticks, peak unqueried events, retained monitor sample count and
completed pruning horizon, so the workload's actual behavior is visible.

Criterion measures 4,096 source events per logical iteration, with ten flat
samples, one second of warmup and a five-second measurement target per case.
Runtime and monitor state are **not reset between samples**. Each timed callback
includes event generation/reference work, periodic queries/validation, monitor
operations, any residual final query and a final live-replay drain. This final
benchmark-only idle wait establishes that submissions were processed before
timing ends; periodic queries never wait for global idle. A final monitor sample
at each callback boundary is included as additional monitoring work. Startup,
shutdown and Criterion's analysis are outside timing.

Throughput comes from Criterion's timed source-event count, not from monitor
rates spanning warmup and gaps between callbacks. Tests use a supplied virtual
clock to check cadence, missed ticks, bounded work, partial blocks, fanout and
persistent cumulative state. A separate rollover test observes completed native
pruning and verifies monitor history stays bounded at 120 samples. Every query
checks counts and a canonical hash before confirming monitor progress.

This workload includes generation, pruning and periodic reads; its throughput
should not be compared directly to the prepared-input `pipeline` benchmark.

### Worker-status coverage

Run `cargo test -p contime-monitor --features runtime` and
`cargo bench -p contime-monitor --features runtime --bench workers`.

The `workers` benchmark measures ready-response collection for 1/4/64 workers,
real Runtime request-to-complete latency with empty/128-store configurations,
and sustained 1/4-worker throughput with periodic status collection enabled or
disabled. The sustained fixture uses a fixed counter payload with evolving
timestamps/IDs, no hashes, fanout, reference model, late traffic, or simulator.
Advancement and 4,096-event insertion-receipt groups are identical in both modes;
20-Hz collection is checked at group boundaries, so it is not a hard schedule.
Timing includes source generation/submission, equal final replay drain, and
status requests/collection when enabled. Startup, shutdown, and final counter
validation are excluded. One Runtime survives Criterion warmup and measurements.
History retention is effectively unbounded; accumulated storage and checkpoint
allocation remain genuine execution costs. These results measure this source
shape, not maximum worker-only processing capacity. `CONTIME_MONITOR_REVERSE=1`
reverses mode order. Tests verify real Runtime collection does not advance its
target and that ordinary counter queries remain correct; fake-source tests
cover partial/invalid streams, clock errors, cancellation, and bounded history.
