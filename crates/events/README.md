# contime-events

`contime-events` is an isolated, passive event-history store. It owns no
threads, channels, snapshots, checkpoints, worker scheduling, memory limits,
or ConTime orchestration.

The crate is not connected to the root `contime` crate or any other ConTime
subcrate. A future orchestrator may adapt this crate's independent API to the
worker boundary.

## Initial scope

- Accept any event ownership representation selected by the consumer.
- Deduplicate retained events by event ID, independently of timestamp.
- Append canonically ordered events to a `VecDeque`.
- Store out-of-order events in a `BTreeMap`.
- Iterate both stores as one canonical `(time, event ID)` history.
- Track the earliest timestamp from which checkpoint replay must resume.
- Iterate from that dirty timestamp while retaining its complete timestamp
  bucket.
- Expose basic length, latest-key, and storage-count inspection.

Horizon pruning, memory accounting, batch insertion, checkpoint
materialization, and integration tests are intentionally deferred.

## Public boundary

Consumers implement the local `Event` trait by providing an ordered time and a
`u128` event ID. The time type's `Default` value is its zero timestamp.
`EventHistory<E>` stores `E` directly as its consumer-selected ownership type
and is the per-snapshot state object. It does not store a snapshot
ID; the eventual worker owns the mapping from snapshot IDs to histories.

An empty history starts dirty at zero. Its first unique event sets the dirty
timestamp to that event's time, and subsequent unique events may move the
timestamp earlier. Duplicate IDs do not change it. `mark_replayed` moves the
boundary to the latest retained timestamp, conservatively preserving the last
same-timestamp bucket for the next replay.

## Benchmark snapshot

Local release-mode Criterion results on 2026-08-29:

| Insertion workload | 1,000 events | ns/event | Events/s |
| --- | ---: | ---: | ---: |
| Ordered append | 7.045 us | 7.04 | 142.0 million |
| Late tree insertion | 43.082 us | 43.08 | 23.2 million |
| Duplicate rejection | 5.300 us | 5.30 | 188.7 million |

Insertion fixtures and final history destruction are outside the timed
section. The measurement includes event-ID deduplication, event-key creation,
dirty-time maintenance, and the selected storage operation. Duplicate
rejection includes dropping the rejected incoming `Arc`.

| Iteration workload | 1,000 events | ns/event | Events/s |
| --- | ---: | ---: | ---: |
| Full history, 0% late | 1.375 us | 1.38 | 727.3 million |
| Full history, 10% late | 1.509 us | 1.51 | 662.7 million |
| Full history, 50% late | 2.068 us | 2.07 | 483.5 million |
| Dirty range over all ordered events | 2.450 us | 2.45 | 408.2 million |

The dirty-range case includes locating the deque boundary, constructing both
range cursors, and visiting all 1,000 events. Direct traversal of the same
ordered deque measured 349.84 ns, providing a lower-bound reference for the
merged iterator abstraction.

## Integration benchmark snapshot

The public-API integration benchmark measures batches of 1,000 events behind
a benchmark-local `Arc` wrapper. Production event storage does not prescribe
that wrapper. Event construction, `Arc` allocation, fixture preparation,
and fixture destruction are outside the timed section. Each result includes
moving every wrapper from the prepared input vector through `EventHistory::insert`;
duplicate insertion also includes dropping the rejected incoming wrapper.

Local release-mode Criterion results on 2026-08-29:

| Event size | Ordered 1,000 | Ordered ns/event | Ordered events/s | Late 1,000 | Late ns/event | Late events/s | Duplicate 1,000 | Duplicate ns/event | Duplicate events/s |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 32 bytes | 7.685 us | 7.68 | 130.1 million | 43.882 us | 43.88 | 22.8 million | 5.305 us | 5.30 | 188.5 million |
| 64 bytes | 7.579 us | 7.58 | 131.9 million | 43.536 us | 43.54 | 23.0 million | 5.440 us | 5.44 | 183.8 million |
| 208 bytes | 8.500 us | 8.50 | 117.6 million | 49.300 us | 49.30 | 20.3 million | 6.181 us | 6.18 | 161.8 million |
| 1,008 bytes | 7.714 us | 7.71 | 129.6 million | 46.965 us | 46.97 | 21.3 million | 5.842 us | 5.84 | 171.2 million |

The payload remains behind an `Arc` and is not copied during insertion, so the
results do not grow proportionally with event size. Ordered insertion is
primarily retained-ID hashing plus deque append; late insertion additionally
pays for ordered B-tree insertion.

The representative 208-byte iteration cases produced:

| Iteration workload | 1,000 events | ns/event | Events/s |
| --- | ---: | ---: | ---: |
| Full history | 1.364 us | 1.36 | 733.0 million |
| Dirty range over all events | 2.449 us | 2.45 | 408.3 million |

Run the integration suite with:

```bash
cargo bench --manifest-path crates/events/Cargo.toml --bench events
```
