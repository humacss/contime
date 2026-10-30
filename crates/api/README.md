# contime-api

`contime-api` isolates the work performed at ConTime's public input boundary.
Its default channel helpers do not know what receives or processes their output.

Runtime uses the default helpers to submit its protocol messages. The optional
`manual` feature adds callback-based request adapters for the worker contracts;
it owns no worker, storage, queue or processing loop. The root manual facade
composes those adapters with Core storage and the thin manual driver.

Default channel helpers use Crossbeam. Enabling `manual` also depends on
`contime-worker`; callbacks may capture thread-local state and never wait on a
channel. They run only after a worker begins processing their request. Empty
queries complete with an empty vector; dropping unprocessed requests does not
invoke callbacks. Callbacks must not block waiting for the same worker.

The synchronous helpers here are independent functions. The root ConTime instance's
`apply` and `advance_to` methods enqueue work asynchronously. In the composed
pipeline, closing an apply rejection stream acknowledges insertion; it does
not establish replay completion.

## Responsibilities

`send` converts each input into the caller-selected input type, collects one
ordinary `Vec<I>`, and asks the caller-selected `ApplyOutput` implementation to
construct the message forwarded through an opaque output channel. `InputBatch`
remains an optional default implementation. The API does not prescribe whether
`I` is an owned value, shared
pointer, tracked pointer, or another ownership type. The batch owns one
rejection sender. An empty input sequence forwards nothing and drops the
sender immediately.

`apply` owns the rejection channel, delegates input forwarding to `send`, and
collects rejection messages until every downstream rejection sender has been
dropped. It sorts and deduplicates identical `(event_id, reason)` pairs before
returning them. Rejection reasons are generic; a downstream core or consumer
defines their meaning while this crate only requires them to be ordered.

The crate does not inspect snapshot IDs, flatten inputs by route, hash routes,
partition work, manage memory, or process events. Its output contract is
defined locally; it does not know that a router exists. An orchestrator can
implement `ApplyOutput` on a type that also implements another crate's input
trait, making the handoff compile-time checked without an API dependency.

Snapshot and event-history queries use the same ownership-neutral boundary.
Asynchronous calls forward a caller-owned response sender. Synchronous wrappers
collect result batches until every downstream sender clone is dropped.
Snapshot queries return only found boxed snapshots; event queries return owned
handles selected by the consumer's output type.

Horizon advancement follows the same closure contract. `send_advance_to`
forwards one timestamp and caller-owned completion sender. `advance_to` owns
that channel and waits until every downstream sender clone has been dropped;
there is no acknowledgement payload.

Snapshot replay listeners use only an asynchronous boundary.
`send_listen_snapshots` collects one watched timestamp and the requested
snapshot-ID set, then forwards them with a consumer-owned notification sender.
The API does not create the channel,
interpret listener notifications, or wait for distributed registration. Empty
registrations forward nothing and drop the supplied sender.

## Verification

Run the unit tests from the ConTime repository root:

```bash
cargo test --manifest-path crates/api/Cargo.toml --lib
```

Run the inline Criterion benchmarks separately:

```bash
cargo test --manifest-path crates/api/Cargo.toml --release --lib benchmark_send -- --ignored --nocapture
cargo test --manifest-path crates/api/Cargo.toml --release --lib benchmark_apply -- --ignored --nocapture
cargo test --manifest-path crates/api/Cargo.toml --release --lib benchmark_send_listen_snapshots -- --ignored --nocapture
```

## Benchmarks

Listener forwarding of one timestamp and 1,000 already-collected snapshot IDs
measured **205.28 ns**, or approximately **0.205 ns per ID**, on 2026-09-02.
The snapshot-ID vector and both channels are prepared outside the timed routine. Like the apply
send benchmark, consuming and recollecting the owned vector can reuse its
allocation; this measurement describes the API boundary only and excludes
router partitioning, worker registration, and notification delivery.

The following medians were recorded locally in one release-mode run on
2026-08-31. The fixtures have compile-time assertions fixing their complete
sizes at exactly 32, 64, 208, and 1,008 bytes.

| Event bytes | Ownership | Inputs | Total | ns/input | Inputs/s |
| ---: | :--- | ---: | ---: | ---: | ---: |
| 32 | owned | 1 | 103.87 ns | 103.87 | 9,627,419 |
| 32 | shared | 1 | 110.42 ns | 110.42 | 9,056,330 |
| 64 | owned | 1 | 104.08 ns | 104.08 | 9,607,994 |
| 64 | shared | 1 | 110.42 ns | 110.42 | 9,056,330 |
| 208 | owned | 1 | 104.81 ns | 104.81 | 9,541,074 |
| 208 | shared | 1 | 110.78 ns | 110.78 | 9,027,983 |
| 1,008 | owned | 1 | 103.68 ns | 103.68 | 9,644,290 |
| 1,008 | shared | 1 | 110.87 ns | 110.87 | 9,019,572 |
| 32 | owned | 1,000 | 87.875 ns | 0.08788 | 11.38 billion |
| 32 | shared | 1,000 | 483.52 ns | 0.48352 | 2.068 billion |
| 64 | owned | 1,000 | 85.384 ns | 0.08538 | 11.71 billion |
| 64 | shared | 1,000 | 495.15 ns | 0.49515 | 2.019 billion |
| 208 | owned | 1,000 | 105.48 ns | 0.10548 | 9.481 billion |
| 208 | shared | 1,000 | 506.20 ns | 0.50620 | 1.975 billion |
| 1,008 | owned | 1,000 | 130.84 ns | 0.13084 | 7.642 billion |
| 1,008 | shared | 1,000 | 522.25 ns | 0.52225 | 1.915 billion |

Each `send` case prepares the input vector and both Crossbeam channels outside
the timed routine. The measurement includes generic identity conversion,
collection into the forwarded vector, and one real output-channel send. It
excludes downstream receipt, input construction, and destruction of the queued
output. Rust can reuse an owned `Vec` allocation while collecting its consumed
iterator, which explains the exceptionally small amortized batch figures.

Both ownership strategies are effectively free at this boundary compared with
downstream routing and processing. The API result does not justify either
ownership strategy; it demonstrates that the generic API does not introduce a
payload-sized copy or allocation. Router fan-out is the operation that makes
shared ownership materially valuable.

`api/apply/no_rejections` measured **107.49 ns**. It includes creation and
closure of the internally owned rejection channel plus empty rejection
collection. Its `send` dependency is stubbed, so it does not include input
normalization, output-channel transport, or downstream work.

Query boundary results recorded on 2026-09-01 include one real request/response
round trip to a minimal downstream thread and synchronous result collection:

| Results | Snapshot query | Event query |
| ---: | ---: | ---: |
| 1 | 1.534 us | 1.023 us |
| 10 | 2.325 us | 1.014 us |
| 100 | 11.01 us | 1.409 us |
| 1,000 | 34.32 us | 3.724 us |

The snapshot fixture allocates one boxed value per result downstream. The event
fixture returns one contiguous vector, so the difference primarily describes
result ownership and allocation rather than request construction.

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

### `query` · criterion · all-features

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `api/query/event_results/1` | 744.731 ns | 737.161 ns–751.056 ns | 100 | — |
| `api/query/event_results/10` | 696.071 ns | 693.450 ns–699.875 ns | 100 | — |
| `api/query/event_results/100` | 891.195 ns | 888.425 ns–895.005 ns | 100 | — |
| `api/query/event_results/1000` | 1.582 µs | 1.577 µs–1.594 µs | 100 | — |
| `api/query/snapshot_results/1` | 893.826 ns | 888.548 ns–895.843 ns | 100 | — |
| `api/query/snapshot_results/10` | 1.366 µs | 1.360 µs–1.374 µs | 100 | — |
| `api/query/snapshot_results/100` | 6.057 µs | 6.020 µs–6.107 µs | 100 | — |
| `api/query/snapshot_results/1000` | 29.559 µs | 29.523 µs–29.601 µs | 100 | — |

### `contime_api` · inline · all-features

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `api/advance_to/immediate_completion` | 84.741 ns | 84.688 ns–84.793 ns | 100 | — |
| `api/apply/no_rejections` | 80.885 ns | 80.744 ns–80.984 ns | 100 | — |
| `api/send_advance_to` | 87.213 ns | 87.172 ns–87.265 ns | 100 | — |
| `api/send_listen_snapshots/1000_ids` | 159.551 ns | 158.441 ns–160.638 ns | 100 | — |
| `api/send/owned/1008_bytes/1000_inputs` | 94.667 ns | 93.567 ns–96.316 ns | 100 | — |
| `api/send/owned/1008_bytes/1_inputs` | 79.839 ns | 78.099 ns–81.077 ns | 100 | — |
| `api/send/owned/208_bytes/1000_inputs` | 77.841 ns | 77.016 ns–79.317 ns | 100 | — |
| `api/send/owned/208_bytes/1_inputs` | 80.593 ns | 78.340 ns–81.207 ns | 100 | — |
| `api/send/owned/32_bytes/1000_inputs` | 69.361 ns | 68.902 ns–69.528 ns | 100 | — |
| `api/send/owned/32_bytes/1_inputs` | 80.606 ns | 79.674 ns–81.642 ns | 100 | — |
| `api/send/owned/64_bytes/1000_inputs` | 67.904 ns | 67.524 ns–68.268 ns | 100 | — |
| `api/send/owned/64_bytes/1_inputs` | 81.044 ns | 79.893 ns–81.854 ns | 100 | — |
| `api/send/shared/1008_bytes/1000_inputs` | 389.387 ns | 387.113 ns–405.562 ns | 100 | — |
| `api/send/shared/1008_bytes/1_inputs` | 84.616 ns | 83.017 ns–86.215 ns | 100 | — |
| `api/send/shared/208_bytes/1000_inputs` | 379.946 ns | 379.123 ns–380.995 ns | 100 | — |
| `api/send/shared/208_bytes/1_inputs` | 85.843 ns | 83.606 ns–86.698 ns | 100 | — |
| `api/send/shared/32_bytes/1000_inputs` | 375.710 ns | 373.724 ns–377.535 ns | 100 | — |
| `api/send/shared/32_bytes/1_inputs` | 85.161 ns | 84.315 ns–85.777 ns | 100 | — |
| `api/send/shared/64_bytes/1000_inputs` | 375.646 ns | 375.182 ns–376.624 ns | 100 | — |
| `api/send/shared/64_bytes/1_inputs` | 85.227 ns | 84.415 ns–86.273 ns | 100 | — |
