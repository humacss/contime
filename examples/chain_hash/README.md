# Chain Hash Verification

This diagnostic example submits events out of order and checks the hash chains returned by ConTime queries. It runs until you quit or send a termination signal:

```bash
cargo run -p contime --example chain_hash -- --counters 6 --workers 4 --events-per-second 100
```

`--counters` chooses how many histories to display; `--workers` chooses the worker thread count. Both default to 4. `--events-per-second` sets the target production rate across all histories (default 10, positive integers up to 1,000,000,000). Random spacing varies around that target. The achieved rate depends on hashing, queries, scheduling, and submission costs; the dashboard shows the actual production average. An event targeting two histories counts as one produced event.

## Out-of-Order Delivery

The producer creates events with strictly increasing nanosecond timestamps and IDs. The main thread independently extends its reference chains in that canonical order. Events are collected into groups for roughly 100 milliseconds, then submitted in **reverse order**. At low rates a group waits for at least two events; at high rates it is capped at 4,096. Every group shares a primary history so reversing it changes arrival order within that history. Every third event also targets a second history when at least two are configured.

A valid query made during incomplete delivery can include a newer event while an earlier event is still missing. Its digest need not match any prefix of the complete reference chain. To make the prefix check meaningful, this example polls each group's insertion completion channel and queries through the latest contiguous, fully admitted group. Closure acknowledges admission on every route, not completed live replay. The producer continues independently; the UI never waits for global idle. Strictly increasing timestamps prevent a newer, incomplete group from sharing the query boundary's timestamp.

The dashboard shows the submitted count, out-of-order arrival count, and the timestamp through which it is querying. Generation, submission, and verification are separate stages, so pending work includes events buffered for reversed delivery.

## Checking the Chain

Each snapshot maintains a BLAKE3 hash chain:

```text
next_hash = hash(previous_hash, event_contents)
```

The initial digest is 32 zero bytes. Each step needs only the previous digest and the current event. The event encoding includes its ID, timestamp, increment, and target IDs. Checkpoints retain the digest with the rest of the state, so pruning older events does not reset the chain.

For each query result, the checker searches its retained reference chain from the beginning for the full returned digest. A match must also have the expected event count and numeric total. It discards entries **before** the match, retaining the matched digest as the next baseline and all newer, unverified entries. Repeating a query at the same position therefore remains valid. The expected chain is built from generated events, never from queried state.

The UI shows **Last valid hash** and **Latest produced** for every history, alongside produced/verified event counts and pending counts. Select a history to see both complete digests in the footer. The table uses shortened digests; all comparisons use the complete 256-bit values. **Matches** means the queried state was found in the retained chain; **No events** is the initial state.

If no digest matches, or a matched digest has an inconsistent count or total, the example stops its producer, restores the terminal, shuts down ConTime, and returns an error with a nonzero exit status. Unexpected admission rejections are fatal too: the reference expects this generated stream to be accepted, so silently dropping a rejected event would invalidate it. Events outside the retained window remain subject to ConTime's normal rejection policy.

First-query delays measure generation until the event has appeared in a verified query on every targeted history. They include grouping, admission, and the 20 Hz query cadence, rather than measuring worker execution time. This is a diagnostic example, not a throughput benchmark.

The tests also materialize a newer event before inserting an older one, verify repaired replay, and confirm that the resulting digest survives pruning. Fixed-size hash matching is a practical integrity check rather than a mathematical proof: collisions are theoretically possible.

Scroll with the arrow keys or Page Up/Down when there are more histories than fit on screen. The display requires at least 96 columns and 17 rows. Press `q`, Escape, or Ctrl-C to stop and restore the terminal. Use `--help` for the options.
