# How ConTime works

[Overview](../../README.md) · [Using ConTime](../usage/README.md) · [Workspace](../workspace/README.md)

ConTime builds continuous-time state from two concepts you define: **snapshots**, which represent state, and **events**, which describe changes to that state.

You define the data and how events change it. ConTime maintains the history needed to reconstruct that state at a requested time.

## Snapshots

A snapshot represents state at a particular point in time.

For example, a snapshot could describe a sensor's latest reading. The snapshot type defines what data is stored, while a snapshot ID identifies which sensor the data belongs to.

ConTime maintains an independent history for each snapshot ID. The same sensor can therefore have different snapshot state at different times, and you can ask for its state at time `T`.

## Events

An event contains timestamped data that can change one or more snapshots. For example, a new sensor reading updates the state belonging to that sensor.

Your input implementation gives the event an ID, a timestamp, and the snapshot IDs it should affect. Your application logic defines how the event changes those snapshots.

An event can affect several histories. A sensor reading might update both the individual sensor's state and a shared summary of readings from multiple sensors.

## Applying Events

Applying events means taking snapshot state and updating it using the events that belong at that time.

Suppose a sensor's state contains its latest reading and the number of readings received. Your application logic might replace the stored reading and increment the count whenever a new reading arrives.

ConTime supplies events in chronological order, grouped into complete timestamp batches. Your application logic processes each batch to produce the next snapshot state. Events at the same timestamp are ordered by event ID.

Events do not need to arrive in that order. If a reading arrives late, ConTime reconstructs the affected history with that reading in its proper place, then reapplies the later events. Previously calculated state may change as a result.

Resubmitting an identical event with the same retained event ID does not apply it twice. Given the same retained events, stable IDs, and deterministic application logic, the resulting state is independent of arrival order.

## Queries

You generally use ConTime by submitting events and querying the state they describe.

A query asks for one or more snapshots at a particular time. For example: what was sensor 42's state at time `T`?

ConTime reconstructs the requested state by applying the relevant events through `T`, including events exactly at that timestamp. Events after `T` do not contribute to the result.

The result describes state at the requested time. Querying an earlier time does not rewind the running ConTime instance or replace its current state.

Queries use admitted, retained history. Submitting an event is asynchronous, so a query does not guarantee that earlier submissions have finished processing. Use the completion response or progress notification corresponding to the operation you need to observe. Insertion acknowledgements establish insertion, not completed live replay.

## Checkpoints

Replaying an entire history for every query would become increasingly expensive. ConTime keeps **checkpoints** to avoid that repeated work.

A checkpoint is saved snapshot state at a particular time. It provides a starting point for reconstructing later state.

To answer a query at time `T`, ConTime starts from an appropriate checkpoint and applies the remaining events through `T`. It only needs to replay the part of the history after that starting point.

If a late event changes history, checkpoints affected by that event must be recalculated. Earlier valid checkpoints can still be used.

Checkpoint spacing controls the tradeoff between storing more saved states and replaying more events. Checkpoints are an optimization; your event application logic still determines the state.

## Routers and Workers

ConTime distributes independent snapshot histories across workers so that they can be processed concurrently.

A **router** uses the destination snapshot IDs supplied by your input implementation to send work to the appropriate workers. All work belonging to a particular snapshot history goes to the same worker. An event affecting several histories can be routed to several workers.

A **worker** owns its assigned histories, applies their events, maintains their checkpoints, and serves queries for their state. With the managed Runtime, each worker runs on its own thread.

Workers process replay in bounded steps and service incoming messages between those steps. This lets a busy history make progress while other submissions and queries are also handled.

Manual uses the same worker on the calling thread, with no router or background
worker thread. The host calls `process()` between its other tasks. See the
[Manual walkthrough](../usage/README.md#run-contime-manually).

The managed flow is:

```text
submit events
    -> admit and batch immediately available input
    -> route by destination snapshot
    -> workers update their histories

query snapshots at time T
    -> route each request to the owning worker
    -> reconstruct the requested state
    -> return the results
```

## Advancement and Retention

ConTime keeps a processing target that you move forward with `advance_to`.

That target controls how far live event application should progress. Future events can already be stored and queried before live processing reaches them. A query reconstructs state independently; it does not advance the processing target.

ConTime also retains a configurable amount of history. As time advances, it can discard older events and checkpoints once coordinated progress establishes that they are safe to prune.

Pruning preserves the snapshot state needed as a starting point for the remaining history. Queries cannot reconstruct history that has already been discarded.

Advancement requests are asynchronous. Processing observations and pruning notifications expose their respective progress boundaries. Idle waiting is a helper for tests and benchmarks, where input can be controlled; running applications should not rely on the system becoming idle.
