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
experiments. Native behavior and public submission APIs are unchanged.

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
cost; they do not measure end-to-end latency or resolve the previously observed
worker-path throughput regression. Use the end-to-end probes under
`benches/scaling` to measure those effects.
