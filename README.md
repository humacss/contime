# contime

Continuous-time state reconciliation for Rust.

You have an event stream — maybe over the network, maybe from disk, maybe from another process.  

Events arrive out of order, duplicated, or with gaps.  

Sometimes a full snapshot arrives late.  

Clients predict and diverge.

And you need to answer one question instantly, from any async task:

**“What is the state of snapshot X at time T?”**

## Continuous Time Queries

`contime` is a tiny (~450 LOC target), single-dependency engine that turns an unreliable stream of events + occasional snapshots into a true continuous-time query interface:

```text
let (snapshot, update_rx) = state.at(snapshot_id, time);
let (update_rx) = state.subscribe_from(id, time);
```

That’s it.  

### Design Goals

- Exactly-once application of events
- Graceful handling of out-of-order events
- Snapshot divergence detection with optional compensation callback  
- Per-snapshot continuous-time queries in **O(log N)** 
- Subscribe to future snapshot updates starting from any point in time
- Strictly bounded memory and channels (no unbounded growth)
- Works everywhere: Windows, Mac, Linux, WASM

### Intended use cases

- Multiplayer game servers & clients (lag compensation, reconciliation, replays)  
- Security/telemetry pipelines that need historical queries  
- Low-latency trading, robotics, collaborative editing 
- Any system that already has an event stream and wants continuous-time reads

### Current status

Work in progress — being extracted from a high-throughput game server.

Aiming for only one dependency:

```toml
tokio = { version = "1.37", features = ["rt", "sync", "time"] }
```

### License

MIT OR Apache-2.0