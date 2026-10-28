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

Feature sets: `all-features` is the workspace feature union, including optional
worker counters; `default` disables optional counters; `status-no-counters`
enables Monitor's `runtime` feature without counters. Each table states the
actual sample count. Inline tests may contain multiple Criterion cases.

Benchmark IDs such as `disabled` or `enabled` describe the fixture’s monitoring
mode. They do not change compiled worker-counter features; the feature-set label
on each table identifies those independently.

### `activity` · criterion · all-features

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `monitor/application-activity/1` | 33.197 ns | 33.175 ns–33.226 ns | 100 | — |
| `monitor/application-activity/128` | 617.742 ns | 614.354 ns–620.240 ns | 100 | — |
| `monitor/application-activity/4` | 49.578 ns | 49.463 ns–49.871 ns | 100 | — |
| `monitor/application-activity/4096` | 31.338 µs | 31.118 µs–31.457 µs | 100 | — |

### `arithmetic` · criterion · all-features

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `monitor/arithmetic/checked_add` | 0.758 ns | 0.750 ns–0.761 ns | 40 | — |
| `monitor/arithmetic/checked_add_overflow` | 0.570 ns | 0.569 ns–0.572 ns | 40 | — |
| `monitor/arithmetic/error_display` | 2.945 ns | 2.939 ns–2.949 ns | 40 | — |
| `monitor/arithmetic/rates` | 3.062 ns | 2.437 ns–3.443 ns | 40 | — |
| `monitor/arithmetic/rates_zero_interval` | 0.853 ns | 0.802 ns–0.908 ns | 40 | — |
| `monitor/arithmetic/since` | 0.570 ns | 0.570 ns–0.571 ns | 40 | — |
| `monitor/arithmetic/since_saturating` | 0.571 ns | 0.571 ns–0.571 ns | 40 | — |

### `operations` · criterion · all-features

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `monitor/operations/active_query` | 1.550 ns | 1.544 ns–1.556 ns | 40 | 645.297 M elements/s |
| `monitor/operations/clock_reversed` | 8.135 ns | 8.085 ns–8.284 ns | 40 | 122.926 M elements/s |
| `monitor/operations/complete_query/0` | 7.546 ns | 7.477 ns–7.603 ns | 40 | 132.514 M elements/s |
| `monitor/operations/complete_query/1` | 8.470 ns | 8.436 ns–8.519 ns | 40 | 118.059 M elements/s |
| `monitor/operations/complete_query/120` | 167.720 ns | 166.752 ns–169.052 ns | 40 | 5.962 M elements/s |
| `monitor/operations/complete_query/4096` | 5.408 µs | 5.368 µs–5.444 µs | 40 | 184.897 k elements/s |
| `monitor/operations/confirmed` | 0.467 ns | 0.466 ns–0.468 ns | 40 | 2.141 G elements/s |
| `monitor/operations/default` | 6.390 ns | 6.386 ns–6.402 ns | 40 | — |
| `monitor/operations/first_sample_allocations` | 40.317 ns | 40.186 ns–40.416 ns | 40 | 24.804 M elements/s |
| `monitor/operations/history/1` | 1.901 ns | 1.898 ns–1.904 ns | 40 | 525.983 M elements/s |
| `monitor/operations/history/120` | 54.128 ns | 53.482 ns–54.660 ns | 40 | 18.475 M elements/s |
| `monitor/operations/history/4096` | 2.473 µs | 2.445 µs–2.493 µs | 40 | 404.375 k elements/s |
| `monitor/operations/invalid_prefix` | 1.873 ns | 1.867 ns–1.880 ns | 40 | 533.929 M elements/s |
| `monitor/operations/invalid_token` | 2.660 ns | 2.628 ns–2.679 ns | 40 | 376.006 M elements/s |
| `monitor/operations/last_query` | 2.564 ns | 2.538 ns–2.577 ns | 40 | 389.991 M elements/s |
| `monitor/operations/new` | 6.481 ns | 6.477 ns–6.488 ns | 40 | — |
| `monitor/operations/observed_at` | 0.823 ns | 0.778 ns–0.841 ns | 40 | 1.216 G elements/s |
| `monitor/operations/query_already_active` | 1.604 ns | 1.601 ns–1.609 ns | 40 | 623.613 M elements/s |
| `monitor/operations/record_submission/1` | 1.942 ns | 1.935 ns–1.952 ns | 40 | 514.964 M elements/s |
| `monitor/operations/record_submission/4096` | 1.928 ns | 1.927 ns–1.929 ns | 40 | 2,124.009 G elements/s |
| `monitor/operations/record_submission/64` | 1.933 ns | 1.931 ns–1.935 ns | 40 | 33.102 G elements/s |
| `monitor/operations/sample_cohort_compaction/1` | 38.397 ns | 38.343 ns–38.424 ns | 40 | 26.044 M elements/s |
| `monitor/operations/sample_cohort_compaction/120` | 38.567 ns | 38.453 ns–38.990 ns | 40 | 25.929 M elements/s |
| `monitor/operations/sample_cohort_compaction/4096` | 43.630 ns | 43.152 ns–43.814 ns | 40 | 22.920 M elements/s |
| `monitor/operations/sample_history_rollover/1` | 35.950 ns | 35.913 ns–35.999 ns | 40 | 27.816 M elements/s |
| `monitor/operations/sample_history_rollover/120` | 36.057 ns | 35.974 ns–36.408 ns | 40 | 27.734 M elements/s |
| `monitor/operations/sample_history_rollover/4096` | 40.185 ns | 39.899 ns–40.408 ns | 40 | 24.885 M elements/s |
| `monitor/operations/snapshot/1` | 17.137 ns | 17.118 ns–17.186 ns | 40 | 58.353 M elements/s |
| `monitor/operations/snapshot/120` | 17.088 ns | 17.058 ns–17.149 ns | 40 | 58.519 M elements/s |
| `monitor/operations/snapshot/4096` | 18.655 ns | 18.610 ns–18.764 ns | 40 | 53.604 M elements/s |
| `monitor/operations/start_query` | 3.926 ns | 3.920 ns–3.930 ns | 40 | 254.733 M elements/s |
| `monitor/operations/start_query_through` | 4.066 ns | 4.063 ns–4.070 ns | 40 | 245.934 M elements/s |
| `monitor/operations/submission_overflow` | 1.067 ns | 1.066 ns–1.068 ns | 40 | 937.063 M elements/s |
| `monitor/operations/submitted` | 0.440 ns | 0.432 ns–0.457 ns | 40 | 2.274 G elements/s |

### `pipeline` · criterion · all-features

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `monitor/pipeline/1w-1targets/disabled` | 1.605 ms | 1.600 ms–1.621 ms | 20 | 2.553 M elements/s |
| `monitor/pipeline/1w-1targets/per-64-events` | 1.706 ms | 1.703 ms–1.715 ms | 20 | 2.400 M elements/s |
| `monitor/pipeline/1w-1targets/per-event` | 1.616 ms | 1.593 ms–1.666 ms | 20 | 2.535 M elements/s |
| `monitor/pipeline/1w-3targets/disabled` | 3.764 ms | 3.741 ms–3.794 ms | 20 | 1.088 M elements/s |
| `monitor/pipeline/1w-3targets/per-64-events` | 3.697 ms | 3.459 ms–3.737 ms | 20 | 1.108 M elements/s |
| `monitor/pipeline/1w-3targets/per-event` | 3.499 ms | 3.450 ms–3.536 ms | 20 | 1.171 M elements/s |
| `monitor/pipeline/4w-1targets/disabled` | 904.740 µs | 902.227 µs–908.647 µs | 20 | 4.527 M elements/s |
| `monitor/pipeline/4w-1targets/per-64-events` | 901.757 µs | 886.558 µs–911.796 µs | 20 | 4.542 M elements/s |
| `monitor/pipeline/4w-1targets/per-event` | 921.178 µs | 908.874 µs–931.385 µs | 20 | 4.446 M elements/s |
| `monitor/pipeline/4w-3targets/disabled` | 1.059 ms | 1.048 ms–1.082 ms | 20 | 3.867 M elements/s |
| `monitor/pipeline/4w-3targets/per-64-events` | 1.035 ms | 1.031 ms–1.040 ms | 20 | 3.957 M elements/s |
| `monitor/pipeline/4w-3targets/per-event` | 1.035 ms | 1.032 ms–1.053 ms | 20 | 3.958 M elements/s |

### `sustained` · criterion · all-features

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `monitor/sustained/1w-1targets/disabled` | 2.370 ms | 2.332 ms–2.466 ms | 10 | 1.729 M elements/s |
| `monitor/sustained/1w-1targets/per-event` | 2.581 ms | 2.440 ms–2.705 ms | 10 | 1.587 M elements/s |
| `monitor/sustained/1w-3targets/disabled` | 5.634 ms | 5.369 ms–6.326 ms | 10 | 727.051 k elements/s |
| `monitor/sustained/1w-3targets/per-event` | 6.197 ms | 5.975 ms–7.406 ms | 10 | 660.978 k elements/s |
| `monitor/sustained/4w-1targets/disabled` | 969.635 µs | 909.231 µs–1.018 ms | 10 | 4.224 M elements/s |
| `monitor/sustained/4w-1targets/per-event` | 940.006 µs | 922.458 µs–954.842 µs | 10 | 4.357 M elements/s |
| `monitor/sustained/4w-3targets/disabled` | 1.889 ms | 1.798 ms–2.125 ms | 10 | 2.168 M elements/s |
| `monitor/sustained/4w-3targets/per-event` | 1.838 ms | 1.781 ms–1.977 ms | 10 | 2.229 M elements/s |

### `workers` · criterion · all-features

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `worker-monitor/begin-cancel` | 247.473 ns | 243.164 ns–250.038 ns | 100 | — |
| `worker-monitor/history` | 0.451 ns | 0.449 ns–0.452 ns | 100 | — |
| `worker-monitor/latest` | 0.498 ns | 0.497 ns–0.500 ns | 100 | — |
| `worker-monitor/ready-collection/1` | 301.834 ns | 298.675 ns–305.524 ns | 100 | — |
| `worker-monitor/ready-collection/4` | 359.289 ns | 356.498 ns–363.061 ns | 100 | — |
| `worker-monitor/ready-collection/64` | 2.084 µs | 2.082 µs–2.086 µs | 100 | — |
| `worker-monitor/runtime-request/1w/0` | 890.572 ns | 862.029 ns–1.283 µs | 10 | — |
| `worker-monitor/runtime-request/1w/128` | 1.012 µs | 984.801 ns–1.937 µs | 10 | — |
| `worker-monitor/runtime-request/4w/0` | 2.220 µs | 1.589 µs–3.776 µs | 10 | — |
| `worker-monitor/runtime-request/4w/128` | 1.930 µs | 1.796 µs–3.081 µs | 10 | — |
| `worker-monitor/sustained-fixed-payload/1w/disabled` | 2.276 ms | 2.193 ms–2.647 ms | 10 | 1.799 M elements/s |
| `worker-monitor/sustained-fixed-payload/1w/enabled` | 2.412 ms | 2.301 ms–2.524 ms | 10 | 1.698 M elements/s |
| `worker-monitor/sustained-fixed-payload/4w/disabled` | 803.138 µs | 780.731 µs–891.407 µs | 10 | 5.100 M elements/s |
| `worker-monitor/sustained-fixed-payload/4w/enabled` | 835.974 µs | 780.748 µs–1.399 ms | 10 | 4.900 M elements/s |

### `workers` · criterion · status-no-counters

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `worker-monitor/begin-cancel` | 253.458 ns | 250.172 ns–258.485 ns | 100 | — |
| `worker-monitor/history` | 0.448 ns | 0.447 ns–0.448 ns | 100 | — |
| `worker-monitor/latest` | 0.493 ns | 0.492 ns–0.495 ns | 100 | — |
| `worker-monitor/ready-collection/1` | 264.602 ns | 263.703 ns–265.558 ns | 100 | — |
| `worker-monitor/ready-collection/4` | 330.857 ns | 328.923 ns–335.484 ns | 100 | — |
| `worker-monitor/ready-collection/64` | 2.017 µs | 2.011 µs–2.022 µs | 100 | — |
| `worker-monitor/runtime-request/1w/0` | 840.751 ns | 823.150 ns–915.312 ns | 10 | — |
| `worker-monitor/runtime-request/1w/128` | 1.361 µs | 1.349 µs–1.391 µs | 10 | — |
| `worker-monitor/runtime-request/4w/0` | 5.209 µs | 4.988 µs–5.359 µs | 10 | — |
| `worker-monitor/runtime-request/4w/128` | 3.008 µs | 1.816 µs–5.226 µs | 10 | — |
| `worker-monitor/sustained-fixed-payload/1w/disabled` | 2.134 ms | 2.075 ms–2.294 ms | 10 | 1.920 M elements/s |
| `worker-monitor/sustained-fixed-payload/1w/enabled` | 2.141 ms | 2.116 ms–2.437 ms | 10 | 1.913 M elements/s |
| `worker-monitor/sustained-fixed-payload/4w/disabled` | 737.404 µs | 713.868 µs–791.203 µs | 10 | 5.555 M elements/s |
| `worker-monitor/sustained-fixed-payload/4w/enabled` | 759.965 µs | 698.448 µs–853.142 µs | 10 | 5.390 M elements/s |
