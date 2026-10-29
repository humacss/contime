# Bevy movement presentation

One Bevy 0.19 renderer displays the queried movement scene for Native, WASI,
threaded WASI and Browser. This is an example-only workspace using Rust 1.95;
it does not add Bevy or a newer Rust requirement to ConTime's published crates.

The renderer knows positions, paths, corrections and arena bounds. It does not
generate events, process ConTime work or decide whether state is correct. The
selected workload owns the simulator, worker monitor and independent validation.
Markers ease toward queried positions without predicting ahead. Current cyan
paths cover revised amber paths at exact overlaps. A fixed camera fits the whole
arena, including when the player reaches a wall.

A read-only overlay inside the game window shows clients, workers, configured
average latency and jitter, speed and spawn spacing. It also shows arrived
inputs, reordered arrivals, corrections, in-flight inputs and query updates.
Counts use thousands separators and fixed metric columns. Arrived inputs include
the initial stopped-client inputs; corrections count client histories revised
across query updates, rather than unique inputs. Settings cannot be edited here.

## Run

From the repository root:

```sh
cargo run --release --example launcher
```

Choose Client movement and any environment. Native and both WASI choices open a
Bevy desktop window. Browser compiles the same presentation to WebAssembly and
embeds it in the page. Runtime/Manual selection still chooses how the workload
runs. Counter and Chain hash retain their terminal dashboards.

Install Rust 1.95 for presentation and its browser target when needed:

```sh
rustup toolchain install 1.95.0 --profile minimal --target wasm32-unknown-unknown
```

On Ubuntu, native builds and renderer tests also need the X11 and Wayland
build dependencies for the enabled window backends:

```sh
sudo apt-get install pkg-config libx11-dev libwayland-dev libxkbcommon-dev
```

The launcher reports missing tools; it does not install them. Browser bindings
also need the wasm-bindgen CLI matching the example manifests.

WASD/arrows choose a persistent direction; Space stops. Inputs pass through the
workload's simulated delay. Closing a desktop window, Escape or Q requests a
stop, then waits for the workload to drain and validate. A successful exit needs
both the final validated report and a successful workload process exit. Errors
retain a bounded diagnostic tail. The presenter reaps its child on cleanup.

The browser keeps its existing Start/Pause/Reset controls. Startup settings are
fixed for the run, and reduced-motion preferences disable marker transitions.

## Code

- `src/scene.rs`: shared Bevy camera, walls, paths and markers.
- `src/hud.rs`: shared settings, status and cumulative-count overlay.
- `src/animation.rs`: presentation-only smoothing and fade state.
- `src/bridge.rs`: latest-value report slot and pause/reset state.
- `src/web.rs`: browser exports; JavaScript retains the workload frame budget.
- `src/native.rs`: bounded report/control pipes to the selected workload process.
- `src/main.rs`: desktop window and keyboard/close handling.

Native uses the existing portable workload compiled to a native executable.
WASI uses that workload compiled to the selected WASI target. The frontend never
substitutes native simulation for a selected WebAssembly workload. Both use the
existing versioned worker-report protocol; at most two reports are buffered.
Browser workload and renderer are separate WebAssembly modules, so rendering
stays outside the Manual processing budget, just as the previous canvas drawing did.
Rendering and other example overhead still affect observed performance.

## Verify

```sh
cargo +1.95.0 test --manifest-path examples/support/graphics/Cargo.toml
cargo +1.95.0 clippy --manifest-path examples/support/graphics/Cargo.toml --all-targets -- -D warnings
cargo +1.95.0 build --manifest-path examples/support/graphics/Cargo.toml --release --lib --target wasm32-unknown-unknown
```
