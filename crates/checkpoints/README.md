# contime-checkpoints

`contime-checkpoints` is an isolated apply-time replay and checkpoint store.
It reads canonical events through a local trait, applies them to consumer-owned
snapshot types, and retains cadence checkpoints plus the current tip.

The crate does not depend on `contime`, `contime-events`, `contime-worker`, or
any other ConTime subcrate. The eventual orchestrator will adapt their
independent traits.

## Initial scope

- Read canonical events from their earliest dirty timestamp.
- Acknowledge canonical history only after replay completes successfully.
- Resume from the latest retained checkpoint before the dirty timestamp.
- Replay complete same-time event buckets in canonical order.
- Pass each canonical timestamp bucket through an injectable apply wrapper.
- Retained-history replay calls `ApplyWrapper::replay_event_batch`, whose default
  delegates to `apply_event_batch`. Query and retention reconstruction call only
  ordinary apply. Consumers attach effect emission to the replay hook, keeping
  reconstruction read-only without global suppression flags.
- Allow wrappers to inspect, filter, or partition effective event batches.
- `ApplyInner::apply_event_batch_with(batch, callback)` permits application with
  borrowed caller context. Existing `apply_event_batch` delegates to this hook.
  ConTime still assigns time after nonempty application and records participation
  for empty batches without calling the callback. History counts are unchanged.
  Context is not checkpointed; callbacks must produce identical state during
  live and reconstruction replay, regardless of external cache contents.
- Move the mutable tip forward in place until its event interval is full.
- Preserve a full tip as a fixed checkpoint and append the next tip.
- Correct existing checkpoints in place when late events require replay.
- Truncate obsolete trailing checkpoints after replay commits its final tip.
- Never insert a new checkpoint between existing checkpoints.

Historical snapshot queries clone the nearest checkpoint at or before the
requested time and replay complete canonical event buckets through that time.
The query-local replay does not mutate retained checkpoints or acknowledge
event history. A separate replay anchor preserves complete state through the
last pruned event. Advancement folds every event strictly before the horizon
into that anchor and removes older cadence checkpoints. Queries older than the
anchor return the anchor as a best-effort result. Worker scheduling, event
mutation, completion handling, and memory policy remain outside this crate.

Wrappers may implement `retain_snapshot(snapshot, horizon)` to compact data
inside the reconstructed anchor and every remaining checkpoint during pruning.
It defaults to no-op and never runs during ordinary apply, replay, or queries.
Repeated/older pruning horizons do nothing; a newer horizon invokes the hook
even without new events. Preserve snapshot time and state/replay semantics at
or after the boundary, and do not mutate shared values held by old readers.
The hook is infallible and must not publish effects. Checkpoint keys and event
counts are unchanged. ConTime Core accounts for size changes in its existing
tracked checkpoint update.

## Unit benchmark snapshot

Local release-mode Criterion results on 2026-08-29:

| Unit workload | 1,000 events/updates | ns per event/update | Throughput |
| --- | ---: | ---: | ---: |
| Apply one complete timestamp batch | 294.98 ns | 0.295 | 3.39 billion events/s |
| Manage sequential tip updates | 20.704 us | 20.70 | 48.3 million updates/s |
| Replay one shared timestamp | 1.524 us | 1.52 | 656.3 million events/s |
| Replay 1,000 unique timestamps | 3.797 us | 3.80 | 263.4 million events/s |

Historical query results on 2026-09-01:

| Query workload | Total | Approximate replay cost |
| --- | ---: | ---: |
| Exact checkpoint | 42.40 ns | checkpoint clone only |
| Replay 10 events | 82.52 ns | 8.25 ns/event |
| Replay 100 events | 270.07 ns | 2.70 ns/event |
| Replay 1,000 events | 2.112 us | 2.11 ns/event |

Horizon advancement over a prepared 1,000-event history measured about
68.6 ns when aligned with a cadence checkpoint and 204.4 ns when the anchor
had to fold 50 additional events between checkpoints.

The apply benchmark includes the default wrapper, `ApplyInner`, one effective
snapshot application, snapshot-time advancement, and a consumer loop over all
1,000 event references. Event allocation and reference-vector construction are
outside the measurement.

The checkpoint-store benchmark performs 1,000 independent sequential replay
sessions with an interval of 100. It includes base-checkpoint cloning, interval
accounting, in-place tip movement, and checkpoint appends. Store construction
and final destruction are outside the measurement.

The replay benchmarks include canonical iteration, timestamp grouping,
wrapper application, checkpoint interval handling, and final tip commit. Their
event histories are prepared outside the measurement. Successful replay
acknowledgement is included. The shared-timestamp case performs one apply call;
the unique-timestamp case performs 1,000 apply calls.

Run each inline unit benchmark with:

```bash
cargo test --release --manifest-path crates/checkpoints/Cargo.toml benchmark_apply -- --ignored --nocapture --test-threads=1
cargo test --release --manifest-path crates/checkpoints/Cargo.toml benchmark_checkpoints -- --ignored --nocapture --test-threads=1
cargo test --release --manifest-path crates/checkpoints/Cargo.toml benchmark_replay -- --ignored --nocapture --test-threads=1
```
