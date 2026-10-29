# contime-worker

Use `SnapshotStore` to adapt history storage and `WorkInput` to adapt the
message protocol. `work_messages` drives the calling thread; `MessageWorker`
provides synchronous control without spawning threads. Public API rustdoc
specifies adapter ownership and completion contracts.

Snapshot queries include events at the requested time. Event-history queries
use `[from, to)`, including the lower bound and excluding the upper; missing
histories and empty results release response ownership without sending a result.
Insertion completion precedes replay, while pruning completion follows every
captured history’s forwarding and pruning hooks.

`contime-worker` owns incremental replay scheduling and the blocking message
receive loop. It depends on no other ConTime crate. Runtime supplies concrete
messages and wraps Core's shared storage adapter;
Runtime owns the background threads. Manual drives the same worker directly.

## Message flow

`work_messages(input, store_config, context, registrations, coordination)` runs
on the calling thread. Each history owns one implementation of `SnapshotStore`.
The worker accepts caller-selected message and input types through `WorkInput`,
`ApplyInput`, and `RouteInput`; `ApplyBatch` and `RoutedInput` are optional
transport containers.

The receive loop drains the messages ready at the start of a bounded receive
batch, then grants up to one replay step per received message. Each step handles
one snapshot through the next distinct timestamp bucket in the worker-wide
schedule or the processing target, whichever comes first. One store call may
process multiple event timestamps. Newly queued messages are checked between steps. Queries
remain serviceable, and continuous read traffic cannot starve replay.

An ordered set groups scheduled histories by timestamp, with snapshot ID
breaking equal-time ties. After each replay step, the store reports
its next replay timestamp; finished histories leave the schedule and future
work waits for its target. Stores must finish all replay through the inclusive
step endpoint, including a complete timestamp bucket. Late
insertions invalidate the complete same-time bucket and its suffix. Duplicate
insertions do not invalidate progress. Notifications flush between steps.

The target starts at the time type's default value. `Advance` raises it
monotonically; replay never processes timestamps beyond it. Retained future
inputs remain queryable and do not prevent idle. Snapshot reconstruction does
not publish effects, advance processing, or change scheduled work.

The worker has no independent replay-budget or deadline configuration. Its
message/replay interleaving is fixed by the incremental loop. Retention belongs
to admission coordination; worker pruning requires an explicit authorization.

## Completion, observation, and pruning

Apply completion acknowledges insertion, including any rejections. It does not
mean scheduled replay has finished. The activity receiver accepts independent
`Sender<bool>` subscriptions (`false` = idle, `true` = working). Registrations
are serviced in the idle loop; working is published before dequeue, and idle
only after replay, hooks, notifications, and queued pruning finish. Pass
`crossbeam_channel::never()` when no subscriptions are needed.

Optional `Coordination` reports possible replay work after a fence from every
router in a round. Duplicate and older fences are ignored. After reporting,
live replay pauses until matching resolution; messages and queries remain
serviceable. Resolution supplies the proven pruning horizon, which may be
unchanged. Pending work receives a processing opportunity between consecutive
reports so zero-interval measurements cannot starve replay.

Only `Prune` and `Resolve` authorize history removal. The worker passes the
admission horizon to every insertion, including while forwarding is queued.
Each pruning step forwards and prunes one history, then returns to message
handling. Completion and idle wait for the whole pass. The completed-horizon
callback runs only after every history and retention hook succeeds. A failed
hook or dropped completion sender never reports successful pruning.

## Worker status

`MessageWorker::status(worker_id)` captures worker orchestration gauges.
`WorkInputKind::Status` requests the same report through the message loop with
an explicitly supplied worker index and Crossbeam response sender. Sending the
report never blocks: a full or disconnected response channel drops it.

`WorkerStatus` distinguishes initialized history stores from listener-only
metadata, counts pending and currently due histories, and reports the accepted
processing target, admission horizon, queued pruning passes, and awaited
coordination round. Due work can still be blocked by pruning or coordination.
The horizon records accepted admission policy, not completed pruning. These
are history gauges, not retained-event counts.

The optional `monitor` feature adds cumulative event-to-history application
counts to `WorkerStatus::counters`. The standalone development crate disables it
by default, as does the published `contime` package. Enabling monitoring also
enables this bookkeeping automatically. The status
field always exists and is `None` when counting is disabled. `WorkerCounters`
is always available. Insertions count accepted canonical changes, excluding duplicates and
rejections. Live replay counts raw canonical traversals, including repeated
prefix replay after late input, and excludes queries and retention forwarding.
These are per-history applications, so one input routed to several histories
contributes several insertions. Counts saturate at `u64::MAX` and retain a
`saturated` signal after overflow.

Custom stores can implement `SnapshotStore::process_until_counted` when enabled.
Its default calls normal processing and returns `None`; once any operation
cannot report a replay count, that worker's replay total remains unavailable.
No clock or rates are maintained by the worker.

Capture scans the history slots and due schedule prefix without calling stores,
reconstructing snapshots, processing events, pruning, or publishing hooks.
Reports from separate workers are not a single atomic system snapshot, and a
status request does not fence inputs still queued elsewhere in the topology.

## Synchronous driving

`MessageWorker::new` constructs the same state used by `work_messages` without
spawning threads. `MessageWorker::new_inline` constructs that state without
channel bindings, for an owned browser host or application resource. Both use
the same message handling, replay scheduling, and pruning methods. `handle` delivers a message, `step` processes a scheduled
snapshot, and `prune_step` forwards/prunes a history. A driver must preserve
normal priority: delivered messages, pending pruning, then replay. Runtime's
controlled frontier tests use this interface with real storage adapters.

## Verification and performance

Run from the repository root:

```sh
cargo test -p contime-worker
cargo clippy -p contime-worker --all-targets -- -D warnings
cargo bench -p contime-worker --bench messages
```

The `messages` benchmark checks observable insertion, processing, query,
notification, and pruning results with storage stubs before timing. It measures
worker overhead rather than end-to-end history or checkpoint performance.
The `worker/status` group checks empty, 1-, 128-, and 4,096-slot fixtures
before and after timing. It separately measures direct capture and same-thread
request/response transport, with initialized stores or listener-only metadata.
Store fixtures mix due and future histories; setup is excluded from timing.
Filter it with `cargo bench -p contime-worker --bench messages -- worker/status`.

Compare the same fixture with and without `--features monitor` in separate
builds to measure optional counting overhead. The benchmark store reports its
existing processed-count delta through the counted adapter. Run timing suites
serially; results depend on the fixture and hardware.

Source units are `work.rs` (receive loop and scheduler), `listen.rs` (replay
notifications), `query.rs` (store queries), and `types.rs` (transport and storage
contracts). `tests/incremental.rs` covers ordering, rewinds, queries, target
bounds, activity, fences, duplicates, pruning, and repeated mixed work.

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

### `messages` · criterion · all-features

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `worker/messages/distinct_timestamps/10000/1_snapshot` | 183.242 µs | 182.878 µs–183.555 µs | 20 | 54.573 M elements/s |
| `worker/messages/distinct_timestamps/1000/1_snapshot` | 19.517 µs | 19.416 µs–19.969 µs | 20 | 51.237 M elements/s |
| `worker/messages/insert_future/1000` | 67.510 µs | 38.510 µs–145.492 µs | 20 | 14.813 M elements/s |
| `worker/messages/mixed/100_rounds` | 456.919 µs | 455.504 µs–458.018 µs | 20 | 21.886 M elements/s |
| `worker/messages/prune/1000_snapshots` | 177.393 µs | 176.861 µs–180.055 µs | 20 | 5.637 M elements/s |
| `worker/messages/queries/1000_pairs` | 128.148 µs | 126.320 µs–131.890 µs | 20 | 7.804 M elements/s |
| `worker/messages/same_timestamp/1000/1000_snapshots` | 162.657 µs | 162.171 µs–163.461 µs | 20 | 6.148 M elements/s |
| `worker/messages/same_timestamp/1000/1_snapshot` | 23.170 µs | 20.381 µs–25.148 µs | 20 | 43.159 M elements/s |
| `worker/status/capture/metadata/0` | 4.841 ns | 4.456 ns–5.296 ns | 20 | — |
| `worker/status/capture/metadata/1` | 4.586 ns | 4.572 ns–4.616 ns | 20 | — |
| `worker/status/capture/metadata/128` | 81.956 ns | 80.859 ns–83.856 ns | 20 | — |
| `worker/status/capture/metadata/4096` | 4.168 µs | 4.074 µs–4.215 µs | 20 | — |
| `worker/status/capture/stores/0` | 4.478 ns | 4.439 ns–4.544 ns | 20 | — |
| `worker/status/capture/stores/1` | 6.236 ns | 6.220 ns–6.257 ns | 20 | — |
| `worker/status/capture/stores/128` | 238.547 ns | 237.677 ns–239.221 ns | 20 | — |
| `worker/status/capture/stores/4096` | 9.584 µs | 9.477 µs–9.757 µs | 20 | — |
| `worker/status/request_response/metadata/0` | 23.954 ns | 23.787 ns–24.129 ns | 20 | — |
| `worker/status/request_response/metadata/1` | 24.303 ns | 24.045 ns–24.594 ns | 20 | — |
| `worker/status/request_response/metadata/128` | 97.541 ns | 94.249 ns–98.255 ns | 20 | — |
| `worker/status/request_response/metadata/4096` | 4.055 µs | 4.034 µs–4.116 µs | 20 | — |
| `worker/status/request_response/stores/0` | 24.170 ns | 24.031 ns–24.817 ns | 20 | — |
| `worker/status/request_response/stores/1` | 25.802 ns | 25.679 ns–26.033 ns | 20 | — |
| `worker/status/request_response/stores/128` | 268.024 ns | 260.631 ns–286.889 ns | 20 | — |
| `worker/status/request_response/stores/4096` | 10.618 µs | 9.954 µs–12.405 µs | 20 | — |

### `messages` · criterion · default

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `worker/messages/distinct_timestamps/10000/1_snapshot` | 182.698 µs | 178.589 µs–185.145 µs | 20 | 54.735 M elements/s |
| `worker/messages/distinct_timestamps/1000/1_snapshot` | 18.578 µs | 18.457 µs–18.897 µs | 20 | 53.826 M elements/s |
| `worker/messages/insert_future/1000` | 17.206 µs | 17.096 µs–17.416 µs | 20 | 58.121 M elements/s |
| `worker/messages/mixed/100_rounds` | 513.983 µs | 495.941 µs–563.204 µs | 20 | 19.456 M elements/s |
| `worker/messages/prune/1000_snapshots` | 189.117 µs | 182.872 µs–202.226 µs | 20 | 5.288 M elements/s |
| `worker/messages/queries/1000_pairs` | 131.794 µs | 127.596 µs–137.120 µs | 20 | 7.588 M elements/s |
| `worker/messages/same_timestamp/1000/1000_snapshots` | 178.412 µs | 170.548 µs–197.642 µs | 20 | 5.605 M elements/s |
| `worker/messages/same_timestamp/1000/1_snapshot` | 19.153 µs | 18.783 µs–19.638 µs | 20 | 52.212 M elements/s |
| `worker/status/capture/metadata/0` | 4.359 ns | 4.263 ns–4.485 ns | 20 | — |
| `worker/status/capture/metadata/1` | 4.332 ns | 4.300 ns–4.336 ns | 20 | — |
| `worker/status/capture/metadata/128` | 77.872 ns | 77.667 ns–78.216 ns | 20 | — |
| `worker/status/capture/metadata/4096` | 4.267 µs | 4.193 µs–4.424 µs | 20 | — |
| `worker/status/capture/stores/0` | 4.429 ns | 4.306 ns–4.608 ns | 20 | — |
| `worker/status/capture/stores/1` | 7.188 ns | 6.321 ns–12.414 ns | 20 | — |
| `worker/status/capture/stores/128` | 244.399 ns | 237.691 ns–248.727 ns | 20 | — |
| `worker/status/capture/stores/4096` | 9.523 µs | 9.474 µs–9.720 µs | 20 | — |
| `worker/status/request_response/metadata/0` | 26.098 ns | 25.255 ns–34.845 ns | 20 | — |
| `worker/status/request_response/metadata/1` | 24.570 ns | 24.225 ns–25.626 ns | 20 | — |
| `worker/status/request_response/metadata/128` | 95.293 ns | 95.064 ns–95.624 ns | 20 | — |
| `worker/status/request_response/metadata/4096` | 4.664 µs | 4.456 µs–5.013 µs | 20 | — |
| `worker/status/request_response/stores/0` | 25.096 ns | 24.294 ns–28.870 ns | 20 | — |
| `worker/status/request_response/stores/1` | 25.836 ns | 25.695 ns–26.164 ns | 20 | — |
| `worker/status/request_response/stores/128` | 256.585 ns | 254.416 ns–260.992 ns | 20 | — |
| `worker/status/request_response/stores/4096` | 9.586 µs | 9.308 µs–9.712 µs | 20 | — |

### `contime_worker` · inline · all-features

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `worker/listen/register_1000` | 44.575 µs | 44.118 µs–45.027 µs | 100 | — |
| `worker/listen/replay_batch/1000_snapshots` | 7.491 µs | 7.360 µs–7.707 µs | 100 | — |
| `worker/listen/replay_batch/100_snapshots` | 812.235 ns | 804.866 ns–837.264 ns | 100 | — |
| `worker/listen/replay_batch/1_snapshots` | 43.766 ns | 43.433 ns–45.378 ns | 100 | — |
| `worker/listen/replay/no_listeners` | 1.623 ns | 1.622 ns–1.627 ns | 100 | — |
| `worker/listen/replay/one_nonmatching_collection` | 4.447 ns | 4.439 ns–4.452 ns | 100 | — |
| `worker/query/events/1000_found` | 1.371 µs | 1.353 µs–1.401 µs | 100 | — |
| `worker/query/snapshot/one_found` | 50.802 ns | 50.306 ns–51.274 ns | 100 | — |
