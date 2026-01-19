# contime

`contime` is an in-memory Rust crate that builds queryable continuous-time state from out-of-order event streams while performing reconciliation of out-of-order events.

The engine supports bounded memory, enables concurrent multi-threaded/multi-processor processing, and performs core operations in the 10–100 ns range.

## Continuous Time?

Most systems today work with **discrete state**: "**right now** the state is X".

`contime` takes a different approach: it builds and maintains **continuous-time state**.

This means that state is defined **at every point in time**, not only at the moments when events arrive, but also in between those events, before those events, and after those events.

Now you can ask the question, what was the state at time `T`? 

This also helps handle out of order events cleanly. What if an event comes in late? 

Many systems will struggle to handle this because now you need to reconcile this event with already applied state, and all you have is the current state. You don't know what the state looked like at the time the event was supposed to arrive. 

With continuous-time state applying an event from the past is as natural as applying one in the present. This means we get deterministic state at all times, regardless of the order of the events coming in to the system.

## Target Use Cases

`contime` was originally built for the needs of a high-performance authoritative game server, but it is generally useful for any system that requires true continuous-time state and out-of-order event processing.

Typical good fits include:

**Multiplayer game servers and clients**  
Handle unreliable networks, lag compensation, client-side prediction, anti-cheat reconciliation, and perfect replays

**IoT & sensor network data**  
Reconcile delayed, duplicated, or out-of-order readings from unreliable devices and networks into a consistent, queryable timeline

**Audit trails & lightweight event-sourcing**  
Provide time-travel queries and deterministic historical state without needing a full persistent event store

## How it Works

**contime** works by accepting `Snapshot` and `Event` inputs from the user. 

### Snapshots
A `Snapshot` is a discrete state at a particular point in time, for example the state of a particular character in a game at time `T`. The `Snapshot` defines the shape of the data, and the `snapshot_id` in this case defines the character the data belongs to.

### Events
An `Event` is data that should be applied to one or more `Snapshot`s. `snapshot_id`s can be extracted from the `Event`, and this is then used to apply the event to one particular `Snapshot` and `snapshot_id`. 

When an `Event` is applied to a `Snapshot`, the event modifies the `Snapshot` state. 

The system keeps a list of `Checkpoint`s internally to keep previous state, and can generate the state at time `T` by grabbing the closest `Checkpoint`, and applying all events in order up until time `T`.

### Flow

When an `Event` or `Snapshot` input arrives from the user, it is sent to the `Router`.  

The `Router` extracts the `snapshot_id` from the input, hashes it, and routes everything to the correct `Worker`.

Each `Worker` maintains a unique set of `snapshot_id`s and works in a dedicated thread running lockless code.

The `Worker` applies the input to the continuous time state for that `snapshot_id`, and then notifies the user when state changes and reconciliation is needed.

This design lets `contime` scale across multiple threads and processors with zero lock contention.

## `contime` is

- A continuous-time state reconciliation engine  
- Handles out-of-order and duplicate events  
- Designed for very fast operation on one server using all available processors  
- Is in memory only and designed to never OOM with bounded memory

In short: a high-performance, in-memory, continuous-time reconciliation engine.

## `contime` is not

- A persistent database or storage layer  
- A general-purpose stream processor  
- A real-time scheduler  
- A full event-sourcing system

`contime` is very specialized for one particular use-case. 

It sits in between other systems like persistence, scheduling, and event sourcing. It does not attempt to be a general solution for event processing. 

## Current Status

The crate is currently a work in progress. The API is not stable yet and some core features are still missing:

- Bounded memory
- Reconciliation notifications
- Crate wide benchmarks
- Early exits on apply
- User provided Snapshots inputs

## License
MIT OR Apache-2.0