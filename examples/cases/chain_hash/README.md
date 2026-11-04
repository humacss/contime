# Benchmark: hash-chain verification

<!-- launcher:overview:start -->
Benchmark measures an end-to-end event workload while checking that `contime`
reconstructs each history in the right order. It generates events with increasing
timestamps, delivers them in configurable groups, and maintains an independent
expected hash chain for every snapshot. The result depends on the order of the
events, so a late event that is applied incorrectly changes the digest.

Choose Benchmark in the launcher, select an environment, and configure the load.
You can choose the execution mode, worker and snapshot counts, event rate,
out-of-order delivery, query frequency and retained history. Advanced settings
control fanout, traffic distribution, checkpoints and processing limits. The
launcher forwards these settings to the same workload used by the commands below.

Out-of-order delivery ranges from 0%, which submits a group in timestamp order,
to 100%, which reverses the entire group. Intermediate values reverse the first
fraction of the group. The default is 100%, producing a demanding replay workload.
The dashboard shows throughput over time, worker activity, and verified hashes
as a count and percentage for all histories and for each worker. A mismatch stops
the run and reports the failure.

The measurements include event generation, delivery, monitoring, queries,
validation and rendering as well as `contime` processing. They describe this
end-to-end example, so they should not be compared directly with the isolated
benchmarks under `benches/`. Use a release build when comparing performance.

<!-- launcher:overview:end -->

Run the native terminal version directly with:

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
identifies the snapshot; its `time` is maintained by `contime`.

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

## Benchmark settings

The launcher shows these settings before starting the benchmark. Common options
can also be passed directly to the native example after `--`, or through the
launcher after its example-argument separator. Browser frame controls are launcher
options and go before that separator. For example:

```sh
cargo run --release --all-features --example chain_hash -- --no-setup \
  --workers 4 --snapshots 1000 --events-per-second 10000 --reorder-percent 50
cargo run --release --example launcher -- --environment browser --example benchmark \
  --no-setup --budget-ms 2 --max-steps 1000 -- --reorder-percent 50
```

### Runtime execution

<!-- launcher:setting:execution-runtime:start -->
Runtime starts background worker threads to process snapshot histories. Your application submits work while those workers process independent histories in parallel. Use Runtime when you want contime to manage local workers for you, especially when several CPU cores can share the work.

In this benchmark, choose Runtime to investigate multi-worker throughput. Start with one worker, then try more workers while keeping the snapshot count and other settings fixed. More workers are not automatically faster. Coordination, memory use, the number of active histories and the rest of the example can limit scaling.

Native and WebAssembly / Wasmtime threads support Runtime. The worker count determines how many threads it starts. Runtime workers are local threads. This setting does not set up networked or distributed processing.
<!-- launcher:setting:execution-runtime:end -->

Select this mode with `--execution runtime`. Set workers with `--workers`.

### Manual execution

<!-- launcher:setting:execution-manual:start -->
Manual gives your application control over when contime processes work. The calling thread runs one worker by calling its processing method instead of starting Runtime background workers. This is useful when you have an existing event loop, need a processing budget, or run in an environment that cannot use Runtime threads.

In the browser, the example uses Manual to share time between processing and rendering. Choose Manual in the benchmark to see how the workload behaves when the host drives one worker itself. The host still does event generation, monitoring and verification, so its result is not a measurement of processing alone.

All four environments support Manual. Node and Browser use it exclusively, and the benchmark uses one worker in this mode. Your application can decide which thread calls Manual processing.
<!-- launcher:setting:execution-manual:end -->

Select this mode with `--execution manual`.

### Workers

<!-- launcher:setting:workers:start -->
Each Runtime worker processes its share of independent snapshot histories. More workers allow histories to be processed in parallel, but adding workers also adds scheduling, coordination and memory overhead. Manual always uses one worker.

A useful starting point is one worker, then a few workers, then a number near the logical CPU count available to your process. A logical CPU is a hardware execution thread, which is not always the same as a physical core. Your operating system and other applications also need CPU time. Using more workers than available logical CPUs makes threads compete and may reduce throughput.

Compare confirmed throughput and the worker activity table while changing the worker count. If extra workers stay idle, try more independent snapshots or less skew before concluding that more workers cannot help. The default follows the available logical CPU count. The default snapshot count also changes with workers, so choose a fixed snapshot count to keep the workload comparable.
<!-- launcher:setting:workers:end -->

Set the worker count with `--workers 4`, for example.

### Snapshot count

<!-- launcher:setting:snapshots:start -->
Each snapshot is one independent hash history. The snapshot count is the number of possible destinations for generated events, not the number of events or the number of workers. With a count of 100, the workload can target 100 different histories.

Use fewer snapshots to explore concentrated work on a small set of histories. Try more snapshots to give multiple workers more independent histories to process. The generator does not promise that every snapshot receives events, especially during a short run or with high skew.

More histories can use more memory and make full verification more expensive. The default is 1,000 per worker and follows the worker count until explicitly changed. When comparing worker counts, set a fixed snapshot count so you are not also changing the size of the workload.
<!-- launcher:setting:snapshots:end -->

Set the snapshot population with `--snapshots 100`, for example.

### Target event rate

<!-- launcher:setting:events-per-second:start -->
This sets the offered rate of source events across the whole benchmark, not per worker. A numeric value such as 10,000 paces generation at up to 10,000 events per second. If processing or verification cannot keep up, confirmed throughput can be lower and work may accumulate within the allowed pending window.

Choose a numeric rate when you want to see how the system behaves under a known load. Compare a modest rate with a higher one and watch confirmed throughput, worker activity, outstanding work and memory. A fixed offered rate is not a promise that the application can sustain it.

Maximum is the default. It lets the example adjust offered load using worker progress and probe throughput. Its result includes generation, replay, monitoring and verification. One event sent to several snapshots counts once as a source event, but causes several snapshot applications.
<!-- launcher:setting:events-per-second:end -->

Use `--events-per-second 10000` for a fixed rate or `--events-per-second max` for Maximum.

### Out-of-order delivery

<!-- launcher:setting:reorder-percent:start -->
Events are generated in timestamp order, then the example reverses a portion at the start of each group before submitting it. At 0% the group stays ordered. At 100% the whole group is reversed. At 50% the first half is reversed and the rest stays in order. This is a repeatable group pattern, not a random probability that individual events arrive late.

Use 0% as an ordered baseline. Compare it with 100% to put more pressure on replay and see how throughput and worker replay activity change for the same settings. Intermediate values provide a less extreme pattern. The default is 100%.

Any nonzero setting reverses at least two events when a group contains enough events. A group of one cannot be reordered. Group sizes and how events target histories also affect how much replay occurs, so the percentage does not predict an exact amount of replay.
<!-- launcher:setting:reorder-percent:end -->

Set the amount with `--reorder-percent 50`, for example.

### Queries per second

<!-- launcher:setting:query-hz:start -->
A verification query reads the histories at a completed work boundary and compares counts, totals and full hash digests with the reference. This checks correctness. Worker monitoring, not these queries, tells the producer when a work window is complete.

The setting limits verification frequency. The default is at most 20 checks per second. Try one check per second if you want checks to interrupt the workload less often, or a higher value to see the cost of more frequent verification. The actual frequency can be lower because the example waits for a suitable boundary and a query takes time.

The example pauses new generation and advancement while a captured query is being verified. Frequent checks can therefore reduce measured throughput. A lower setting does not disable validation. The final state is still checked when the run drains.
<!-- launcher:setting:query-hz:end -->

Set the maximum frequency with `--query-hz 1`, for example.

### History retention

<!-- launcher:setting:retention-ms:start -->
Retention controls how much application-time history contime keeps for late events and historical queries. A setting of 10,000 milliseconds requests a ten-second history window. Older history is eventually pruned and is no longer available for historical queries or replay within that window.

Choose a longer window when an application needs to accept later information or query further into the past. Choose a shorter window when recent history is enough and you want to limit retained state. This does not delay event delivery or make every event arrive within the window.

In this benchmark, compare short and long windows to see how retained events, checkpoints, memory use and pruning affect the end-to-end workload. Keep the event rate and other settings fixed. The default is 10,000 milliseconds.
<!-- launcher:setting:retention-ms:end -->

Set retention in milliseconds with `--retention-ms 10000`, for example.

### Checkpoint spacing

<!-- launcher:setting:checkpoint-interval:start -->
A checkpoint saves an intermediate snapshot state. When a late event changes a history, replay can start from a suitable checkpoint rather than rebuilding all retained events from the beginning. The setting is an approximate number of applied events between checkpoints per history, not a time interval.

Use a smaller spacing if late events and replay are common and you want to investigate whether more frequent checkpoints reduce replay work. Use a larger spacing if you want fewer checkpoints and less state to store. The trade-off depends on the workload and the age of late events.

The default is 128 events. Compare a spacing of 32 with 256 while keeping the reorder percentage and retention fixed. More checkpoints also cost memory and creation work. Fewer checkpoints may require longer replay, so neither setting is universally faster.
<!-- launcher:setting:checkpoint-interval:end -->

Set the spacing with `--checkpoint-interval 32`, for example.

### Average snapshots per event

<!-- launcher:setting:average-snapshots-per-event:start -->
A source event can affect one or more distinct snapshot histories. This setting controls the average number of destinations per generated event. It does not require every event to have exactly that number. The default is 1.0, which targets one history per event.

Use a higher average to model events that update several pieces of state and to measure the extra applications and coordination they create. For example, an average of 1.1 with a maximum of two produces roughly one two-target event in ten over time. Set the maximum to at least the average.

The headline rate counts source events, not each target application. At an average near two, the same source-event rate generally creates more snapshot work than at an average of one. Compare worker application counts as well as source-event throughput.
<!-- launcher:setting:average-snapshots-per-event:end -->

Set the average with `--average-snapshots-per-event 1.1`, for example.

### Maximum snapshots per event

<!-- launcher:setting:max-snapshots-per-event:start -->
The maximum is the cap on how many distinct histories one source event may target. The generator chooses destination counts to produce the selected average, but no event exceeds this cap. The default is two, or the snapshot count if that is smaller.

Increase the cap when you want to test events with wider fanout. For example, a maximum of three permits the average to be set above two. The cap must be at least the selected average and cannot exceed the snapshot population.

Raising the cap by itself does not increase the selected average. To measure more multi-target work, adjust the average too. Keep both values fixed when comparing unrelated changes such as worker counts.
<!-- launcher:setting:max-snapshots-per-event:end -->

Set the maximum with `--max-snapshots-per-event 3`, for example.

### Snapshot skew

<!-- launcher:setting:snapshot-skew:start -->
Skew controls how evenly destinations are selected. At zero, traffic is spread evenly on average. As the value rises, lower-numbered histories receive more work. At 100, the example concentrates on the lowest IDs needed for an event's distinct targets.

Use zero to explore a relatively balanced workload. Try a skew of 90 to model hot histories and see whether some workers do much more work than others. A heavily concentrated workload may use fewer independent histories and limit the benefit of extra workers.

The default is zero. Skew does not mean a percentage of active snapshots and does not promise a precise share of events or coverage. Keep the snapshot count, event rate and worker count fixed when comparing skew values.
<!-- launcher:setting:snapshot-skew:end -->

Set the skew with `--snapshot-skew 90`, for example.

### Pending source events

<!-- launcher:setting:max-pending:start -->
For a numeric event rate, this limits source events submitted by the example but not yet confirmed as a completed window through worker monitoring. It is a conservative count. Some of that work may have finished but still be waiting for the next confirmation.

Use a smaller window to limit backlog and memory, or a larger window to see whether having more work in flight keeps workers busy. If the window is too small, the producer may wait even though workers could process more. A larger window can also hold more source events in memory.

Try a pending limit of 10,000 together with a numeric target event rate. The default follows the rate and worker settings. Maximum mode manages its own bounded windows, so the launcher hides this setting in that mode. One event targeting several histories still uses one source-event slot.
<!-- launcher:setting:max-pending:end -->

Set the limit with `--max-pending 10000` together with a numeric `--events-per-second` value.

### Browser processing budget

<!-- launcher:setting:budget-ms:start -->
In the browser, processing shares the animation-frame schedule with rendering and input. This setting is the time in milliseconds the example may spend processing in a frame before it yields. The default is 1 ms.

Try a larger budget if throughput is low and the display remains responsive. Try a smaller budget if processing makes the interface feel sluggish. A larger budget can leave less time for rendering and input. A complete operation is never interrupted, so an operation can exceed the time budget.

The example also stops processing for the frame when it reaches the step limit. Compare these settings together. This setting applies only to Browser.
<!-- launcher:setting:budget-ms:end -->

Pass the launcher option `--budget-ms 2` before the separator for example arguments.

### Browser processing steps

<!-- launcher:setting:max-steps:start -->
This limits processing-loop steps per browser animation frame. A step is one iteration of the example's processing loop, not necessarily one completed source event. The default is 1,000 steps per frame.

Try fewer steps if the interface needs to yield more often. Try more steps if the step limit is reached before the time budget and you want to allow more work. Increasing it will not help if the time budget is already the limiting factor, and fewer steps may reduce throughput.

Processing stops for the frame when either the step limit or time budget is reached. This setting applies only to Browser.
<!-- launcher:setting:max-steps:end -->

Pass the launcher option `--max-steps 500` before the separator for example arguments.

## Read the dashboard

- **Target events/s:** adaptive offered-load setting for the next window; Maximum
  emits it as quickly as the host can, while numeric mode enforces pacing.
- **Confirmed events/s:** the source-event rate confirmed between recent worker
  readings, including the effects of generation, replay and verification.
- **Outstanding:** submitted source events not yet monitor-confirmed. Completed
  windows release this count, so it is conservative between readings.

The worker table reports insertions into individual histories. One event affecting
three snapshots contributes three applications, so these totals are distinct from
the source-event counts. Pending histories are scheduled for processing, including
future work.

The throughput graph compares the recent confirmed source-event rate with the
submission target over the last 30 seconds. The left scale shows event rates and the count of outstanding events. The right
scale shows workload memory. Hover to inspect a reading,
click to pin it, and click outside to release it. The graph describes the
end-to-end example, including generation, monitoring and verification, rather
than measuring contime in isolation.

The graphical dashboard shows verified hashes as a count and percentage for all
configured histories and for each worker, including empty histories. Verification
starts after the first completed check. A newer generated state is not counted as
verified until it is checked. A mismatch stops the workload and leaves the failure
visible. Use the arrow keys or Page Up and Page Down to browse workers.

## Out-of-order delivery and verification

The simulator creates strictly increasing event timestamps and IDs. The host
builds canonical reference state before changing each group for submission.
`--reorder-percent` controls the portion reversed within each group: 0 keeps
it in timestamp order and 100 reverses it completely. The default is 100.
Intermediate values reverse the first chosen fraction, with at least two events
when a group contains two or more. Groups hold at most 1,024 source events. At low rates the host waits for a pair
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
These are live diagnostics including host overhead, not isolated `contime` capacity
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
