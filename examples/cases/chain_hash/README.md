# Chain Hash Verification

Generate events, deliver groups out of order, and verify the complete hash chains
returned by ConTime. Native Runtime and Manual execution combine the simulator iterator
with worker monitoring and adaptive Maximum production:

```sh
cargo run --release --all-features --example chain_hash
```

The startup form retains the existing settings. `--no-setup` starts directly;
q, Escape, Ctrl-C, or a termination signal stops production, confirms the final
window through monitoring, and verifies the final state. Settings are fixed
once the example starts.

Start with [`model.rs`](model.rs) for events, snapshots and the hash operation,
then [`generation.rs`](generation.rs) for the two simulator callbacks. The event
callback creates canonical events; the assignment callback selects identifying
fields from a configured snapshot population. [`../../support/entrypoints/chain_hash.rs`](../../support/entrypoints/chain_hash.rs) selects the
host and translates startup settings. The [shared diagnostic support](../../support/monitored/README.md)
connects Maximum, collection, input, rendering and validation to the generic
environment execution adapter.

## Events, snapshots and the lane

[`model.rs`](model.rs) defines one event (`CounterChanged`) and one snapshot
(`Counter`). These names reflect the numeric counter carried alongside the hash.
An event contains a stable `event_id`, `time`, signed `delta`, and destination
`counter_ids`. Each selected snapshot adds the delta, increments `event_count`,
and extends its full `hash` with the complete event contents. Its `counter_id`
identifies the snapshot; its `time` is maintained by ConTime.

`Input::snapshot_ids` selects the counters, and
`ApplyEvents<CounterChanged> for Counter` defines their update. A multi-target
event extends each selected chain independently. The sum alone cannot detect
reordering; the digest includes the previous digest, so the entire prefix matters.
`create` initializes a zero total, zero count and zero digest; the application
batch then supplies the first chain link.

[`setup.rs`](setup.rs) selects those event/snapshot types and exposes their
counts, totals and digests to shared example tools. It registers no extra lane
and starts no workers. The environment supplies Runtime or Manual execution;
support connects generation, monitoring, validation and presentation. The `Example`
trait used by this adapter is specific to the diagnostic harness.

## Configure the load

```sh
# Slow enough to inspect; reversed pairs still exercise ordering.
cargo run --release --all-features --example chain_hash -- \
  --snapshots 6 --workers 2 --events-per-second 100 --no-setup

# Finite, variable-fanout run with no terminal rendering.
cargo run --release --all-features --example chain_hash -- \
  --headless --duration 2 --snapshots 100 --workers 2 \
  --average-snapshots-per-event 1.5 --max-snapshots-per-event 3
```

Maximum is the default. Worker and snapshot defaults retain one worker per
available logical CPU and 1,000 snapshot identities per worker. Checkpoint
spacing starts at 128 events and retention at ten seconds. Use a numeric
`--events-per-second` for a paced source-event cap. `--snapshot-skew 0..100`
controls concentration; the configured population is possible coverage, not a
guarantee that every snapshot receives work. Average and maximum snapshots per
event control a bounded fanout distribution. Each group keeps a common primary
destination; additional destinations follow cyclically, so reversal matters
within a history.

`--query-hz` controls the maximum frequency of hash checks, performed at completed
window boundaries. `--retention-ms` and `--checkpoint-interval` control storage.
Numeric `--max-pending` remains a source-event window ceiling. Maximum manages
its bounded window automatically. `--help` lists all startup settings.

## Read the dashboard

- **Target events/s:** adaptive offered-load setting for the next window; Maximum
  emits it as quickly as the host can, while numeric mode enforces pacing.
- **Average completed events/s:** cumulative monitor-confirmed source events
  divided by elapsed run time, including generation, live replay, collection,
  and periodic validation overhead.
- **Outstanding:** submitted source events not yet monitor-confirmed. Completed
  windows release this count, so it is conservative between readings.

The header also shows reversed arrival transitions and validation query requests.
The worker table below reports insertions and raw replay in target applications.
One event affecting three snapshots contributes three applications. Late events
can require repeated traversals, so replay totals are not unique-event progress
and are never subtracted to calculate backlog.

The hash table shows last verified and latest produced digests. Differences can
be normal while newer generated state awaits verification. Tab selects snapshot
or worker scrolling; arrows, Page Up/Down, Home and End navigate. Only visible
rows are formatted, but all snapshots are validated. The footer shows both full
digests for the first visible snapshot.

## Out-of-order delivery and verification

The simulator creates strictly increasing event timestamps and IDs. The host
builds canonical reference state before reversing each group for submission.
Groups hold at most 1,024 source events. At low rates the host waits for a pair
when its window allows; one-event ceilings and final single-slot groups cannot
be reversed.

The Maximum controller uses only worker monitor data to establish insertion
and finished live replay for a sealed window. Periodic queries then verify the
count, total, and full digest at that boundary. They do not provide processing
feedback. While a query is pending, new generation and advancement pause, so
later inputs and pruning cannot invalidate the captured expectation. Monitoring,
input, and rendering continue independently of query polling.

Each snapshot maintains a BLAKE3 chain:

```text
next_hash = hash(previous_hash, event_contents)
```

The initial digest is 32 zero bytes. Each step carries the previous digest and
the entire event: ID, timestamp, increment, and target IDs. One ordering mistake
changes subsequent digests. Checkpoints retain the digest when older events are
pruned. All comparisons use 256 bits, even when the table shows a shortened hash.
Hash matching is an integrity check; theoretical collisions remain possible.

Unexpected rejections, missing nonempty snapshots, duplicate/unknown query IDs,
and any count, total or full-digest mismatch terminate the run with an error.
Reference storage is bounded by snapshot population plus the current bounded
production window and one query response. No global idle wait is used.

## Other execution environments

Manual execution uses the same monitored host with caller-driven processing:

```sh
cargo run --release --all-features --example chain_hash -- \
  --execution manual
```

WASI and browser hosts use the same simulator callbacks and monitor-driven
production. WASI uses the shared driver; browser frames use a nonblocking adapter.

## Tests

```sh
cargo test --all-features --example chain_hash
```

Regressions cover fixed/Maximum production, variable fanout, low-rate reversed
pairs, full-digest mismatch detection, and genuinely materialized newer events
followed by late older events. The repaired chain is checked again after pruning.
These are live diagnostics including host overhead, not isolated ConTime capacity
benchmarks.

## Regression coverage

`model_tests.rs` checks that order affects the complete digest and that a cloned
snapshot continues the same chain. `monitored_tests.rs` exercises the active host:
late events repair materialized state, pruning preserves hashes, partial/single-slot
windows complete, and invalid digests terminate validation. Shared reference tests
in `reference_tests.rs` cover canonical timestamp/ID order,
captured prefixes, fanout and invalid/missing/duplicate queried states.

Manual, WASI and browser hosts share simulator callbacks, monitor-driven
production and periodic full-digest checks. Browser processing stays within its
soft frame allowance. WASI's terminal preview is bounded and labelled; every
configured snapshot is validated in the guest.
