# Core snapshot-store integration

Approved design: integrate the new snapshot store with the timestamp-bucket worker;
borrow event iterators through application hooks; initialize from the first accepted
event; reject snapshot queries before the retained horizon; let Apply own event
count updates. Consumers provide a local timestamp wrapper implementing Timestamp
and AdvanceTime. Macro support and runtime migration are deferred.

## Work

- [x] Replace Core's legacy checkpoint adapter with a worker SnapshotStore adapter
  owning the concrete snapshot store and canonical History. Preserve deduplication
  outcomes and use the worker's admission horizon before insertion.
- [x] Implement History's EventStore/InsertEventStore bridge and expose read-only
  event access on snapshots for Core's existing event query API.
- [x] Pass borrowed iterators through EventBatch/ApplyBatch and application hooks;
  retain effect-free queries and live processing/forwarding. Application sees the
  preceding checkpoint count; snapshots Apply advances the canonical count.
- [x] Migrate local unit/integration/benchmark fixtures to timestamp wrappers and
  iterator consumption. Test complete timestamp batches, late replay, duplicates,
  forwarding hooks, pruning, and before-horizon query rejection.
- [x] Run Core all-target tests, focused snapshots tests, lint and formatting;
  run representative integration benchmarks and document results and limitations.

Preserve preexisting dirty work. No commits or downstream runtime edits in this
pass. Core tests may require adapting expectations that explicitly depended on
the replaced scheduling/retention behavior.
