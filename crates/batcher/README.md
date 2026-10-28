# contime-batcher

`contime-batcher` builds bounded transport packets independently of caller
submission sizes. It has no runtime dependencies, owns no threads, and knows
nothing about event time, histories, routing, or storage.

In the native ConTime pipeline it runs on the existing coordinator thread,
**after each original request passes admission and before the routers**. No
additional queue or thread is introduced. Runtime implements the protocol adapters;
the batcher owns packet sizing, splitting, adjacent merging, and bounded draining.

## Contracts

- `Batch` exposes owned items and cloneable segment context. Splitting moves
  items and clones context once per chunk, retaining completion ownership.
- `Batch::can_merge` permits adjacent segments to merge only when their context
  is interchangeable. Runtime compares response channels; distinct destinations
  remain distinct segments even inside the same packet.
- `Batcher::push` immediately dispatches full packets. `flush` dispatches partial
  packets; dropping the batcher releases buffered ownership without dispatch.
- `Request` distinguishes batchable input from control messages. `drain` admits
  each original request before packetization, flushes before returning a control
  boundary, and leaves later messages queued.
- The ready-input callback must be nonblocking. No timer waits for arrivals.
  Empty or disconnected input, exhausted gathering work, and the request limit
  all flush immediately. Dispatch errors propagate without rollback of earlier
  packets.

ConTime uses 1,024 items per packet, at most 1,024 original requests per drain,
and a 50 microsecond soft gathering-work budget. The coordinator supplies the
clock check after each request; a single large submission or admission callback
can exceed that budget. `Batcher::new` accepts a positive packet limit for focused
experiments.

## Isolated verification and measurement

```bash
cargo test -p contime-batcher
cargo bench -p contime-batcher --bench batcher
```

The Criterion matrix measures packetization alone and ready-queue draining,
using a fixed 16,384 items submitted individually or 1,000 at a time. Each case
uses either a shared response context or a distinct `Arc` context per request.
Fixture construction, initial context allocation, and output destruction are
outside the timed routine. Splitting, merging, packet allocation, dispatch into
an in-memory output vector, and consumed input-vector destruction are timed.

The draining case includes request limits and a trailing control boundary but
uses an always-open work-budget callback. It excludes admission, clock reads,
channel synchronization, routers, and workers. These benchmarks isolate batching
cost. Use the end-to-end probes under `benches/scaling` to measure pipeline
throughput and latency.

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

### `batcher` · criterion · all-features

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `batcher/drain/distinct-responses/1` | 89.324 µs | 88.245 µs–91.463 µs | 10 | 183.423 M elements/s |
| `batcher/drain/distinct-responses/1000` | 9.529 µs | 8.922 µs–11.612 µs | 10 | 1.719 G elements/s |
| `batcher/drain/shared-response/1` | 209.673 µs | 203.931 µs–217.076 µs | 10 | 78.141 M elements/s |
| `batcher/drain/shared-response/1000` | 14.169 µs | 13.791 µs–14.588 µs | 10 | 1.156 G elements/s |
| `batcher/packetize/distinct-responses/1` | 69.984 µs | 68.946 µs–79.319 µs | 10 | 234.109 M elements/s |
| `batcher/packetize/distinct-responses/1000` | 9.598 µs | 9.097 µs–11.616 µs | 10 | 1.707 G elements/s |
| `batcher/packetize/shared-response/1` | 193.472 µs | 191.435 µs–200.491 µs | 10 | 84.684 M elements/s |
| `batcher/packetize/shared-response/1000` | 13.578 µs | 12.865 µs–15.074 µs | 10 | 1.207 G elements/s |
