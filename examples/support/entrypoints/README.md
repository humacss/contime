# Native example composition

These entry points select case definitions and connect them to shared support.
They are composition roots, not environment implementations. `contime` startup and
native process lifetime come from the generic environment adapters.

| File | Cargo example | Case |
| --- | --- | --- |
| `chain_hash.rs` | `chain_hash` | Chain hash verification |
| `client_movement.rs` | `client_movement` | Client movement |

The counter entry point is retained as the internal `counter_internal` test target.
The launcher selects the public commands above. Case code lives in `../../cases`; shared
simulation and monitoring composition lives in `../monitored`. Self-contained
API walkthroughs stay beside their cases.
