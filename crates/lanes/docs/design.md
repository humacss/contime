# Isolated Lanes Design

## Boundary

The crate defines only compile-time contracts and direct function dispatch. It
does not own history, checkpoints, workers, threads, routing processes, macros,
or orchestration, and it imports no sibling ConTime crate.

## Pipeline

Every snapshot consumes three conceptual lane sets:

1. Raw lanes contain canonical retained events.
2. Filter lanes are a snapshot-specific borrowed projection of relevant raw
   events.
3. Apply lanes contain exactly the transient events understood by snapshot
   application.

The three sets are equivalent by default. A custom filter may suppress,
reorder, combine, decorate, or replace projected events. Filter output is
transient and never becomes canonical history implicitly.

## Contracts

- Lane families define an event and batch representation for every borrow
  lifetime. This allows generated borrowed batches and avoids payload clones.
- A raw projection creates a filter batch without prescribing its storage.
- A filter emits exactly one apply batch at the original timestamp; an empty
  batch represents total suppression.
- Apply functions consume the filter's output lane type, never the raw type.
- Filters are stateless, deterministic, infallible, and receive no snapshot or
  external context.
- The first routed raw event may initialize a snapshot even when filtering
  suppresses it. Initialization starts from `Default`, preserves the default
  timestamp, and initializes identity only.
- Raw history owns identity, time, counts, deduplication, and retained event
  memory. Filter and apply values are temporary views.
