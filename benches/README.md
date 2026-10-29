# ConTime integration benchmarks

These benchmarks exercise the public ConTime pipeline and example-owned terminal
presentation. Component benchmark results live in each crate's README. Timing
boundaries and fixture definitions are in the corresponding benchmark sources.

Runtime is the public package's default feature. Enable `monitor` to include
worker application bookkeeping; there is no separate counters feature. The dated
results below retain their original feature labels: `counters` and
`status-no-counters` describe the 0.1.0 configuration used for those measurements.
Use `cargo bench -p contime` for the unmonitored root pipeline and
`cargo bench -p contime --features monitor` for a monitored build. The scaling
probes explicitly select the uncounted Runtime.

Run the public pipeline and presentation targets from the repository root:

```sh
cargo +1.90.0 bench -p contime --all-features
cargo +1.90.0 bench -p contime
```

For the whole workspace, use `cargo +1.90.0 bench --workspace --all-features`.
Inline Criterion benchmarks are ignored unit tests and require a separate run:

```sh
cargo +1.90.0 test --release --workspace --all-features --lib benchmark_ -- --ignored --nocapture --test-threads=1
```

Run benchmark suites sequentially. Keep compiler, dependencies, features and
workload settings consistent when comparing measurements.

## Full benchmark run — 2026-10-28

Measured on Apple M3 Pro (11 logical CPUs), Rust 1.90.0, optimized Cargo bench
profile, with unpinned threads and normal desktop background activity. All groups
ran sequentially after compilation. Existing warm-up, sample-count and measurement
settings were retained; no shortened timing overrides were used. These are local
measurements, not production-capacity guarantees. The workspace lockfile uses
Criterion 0.5.1, Crossbeam channel 0.5.15 and Crossbeam utils 0.8.21.
Earlier dated results above
remain historical records and may use different fixtures or settings.

Tables report the **median time per benchmark iteration**, with its bootstrap
95% confidence interval. Rates are derived from that median only where the
benchmark declares a throughput unit; that unit may be source events, target
applications, snapshots, bytes, or bookkeeping calls. It is not automatically
ConTime's event-processing capacity. Thread-startup and allocation boundaries
remain those defined in each benchmark source.

Feature variants were measured in separate sequential runs, not interleaved
paired trials; changing background load can affect their differences.

Historical 0.1.0 feature sets (before monitoring enabled counting automatically):
`all-features` is the workspace feature union, including optional
worker counters; `default` disables optional counters; `status-no-counters`
enables Monitor's `runtime` feature without counters. Each table states the
actual sample count. Inline tests may contain multiple Criterion cases.

### `advance` · criterion · all-features

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `core/advance/1000_histories/anchor/1r_10w` | 198.187 µs | 197.796 µs–199.244 µs | 100 | 5.046 M elements/s |
| `core/advance/1000_histories/anchor/1r_1w` | 124.016 µs | 123.625 µs–124.687 µs | 100 | 8.063 M elements/s |
| `core/advance/1000_histories/anchor/1r_4w` | 127.700 µs | 126.940 µs–128.631 µs | 100 | 7.831 M elements/s |
| `core/advance/1000_histories/anchor/2r_10w` | 207.118 µs | 205.879 µs–208.014 µs | 100 | 4.828 M elements/s |
| `core/advance/1000_histories/clean/1r_10w` | 190.791 µs | 189.996 µs–191.615 µs | 100 | 5.241 M elements/s |
| `core/advance/1000_histories/clean/1r_1w` | 89.562 µs | 89.148 µs–89.910 µs | 100 | 11.165 M elements/s |
| `core/advance/1000_histories/clean/1r_4w` | 118.798 µs | 117.587 µs–120.075 µs | 100 | 8.418 M elements/s |
| `core/advance/1000_histories/clean/2r_10w` | 198.041 µs | 196.958 µs–198.870 µs | 100 | 5.049 M elements/s |
| `core/advance/1000_histories/dirty/1r_10w` | 317.986 µs | 315.322 µs–320.083 µs | 100 | 3.145 M elements/s |
| `core/advance/1000_histories/dirty/1r_1w` | 936.439 µs | 933.216 µs–944.404 µs | 100 | 1.068 M elements/s |
| `core/advance/1000_histories/dirty/1r_4w` | 324.763 µs | 322.382 µs–326.695 µs | 100 | 3.079 M elements/s |
| `core/advance/1000_histories/dirty/2r_10w` | 340.280 µs | 338.713 µs–342.104 µs | 100 | 2.939 M elements/s |

### `advance` · criterion · default

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `core/advance/1000_histories/anchor/1r_10w` | 198.338 µs | 196.230 µs–202.328 µs | 100 | 5.042 M elements/s |
| `core/advance/1000_histories/anchor/1r_1w` | 129.135 µs | 128.807 µs–129.447 µs | 100 | 7.744 M elements/s |
| `core/advance/1000_histories/anchor/1r_4w` | 127.496 µs | 125.874 µs–129.108 µs | 100 | 7.843 M elements/s |
| `core/advance/1000_histories/anchor/2r_10w` | 208.445 µs | 205.717 µs–210.880 µs | 100 | 4.797 M elements/s |
| `core/advance/1000_histories/clean/1r_10w` | 189.696 µs | 188.091 µs–190.919 µs | 100 | 5.272 M elements/s |
| `core/advance/1000_histories/clean/1r_1w` | 93.870 µs | 93.604 µs–94.097 µs | 100 | 10.653 M elements/s |
| `core/advance/1000_histories/clean/1r_4w` | 118.257 µs | 117.029 µs–120.314 µs | 100 | 8.456 M elements/s |
| `core/advance/1000_histories/clean/2r_10w` | 196.647 µs | 194.837 µs–197.960 µs | 100 | 5.085 M elements/s |
| `core/advance/1000_histories/dirty/1r_10w` | 322.300 µs | 320.099 µs–325.060 µs | 100 | 3.103 M elements/s |
| `core/advance/1000_histories/dirty/1r_1w` | 945.222 µs | 940.038 µs–949.514 µs | 100 | 1.058 M elements/s |
| `core/advance/1000_histories/dirty/1r_4w` | 323.288 µs | 321.970 µs–324.265 µs | 100 | 3.093 M elements/s |
| `core/advance/1000_histories/dirty/2r_10w` | 343.591 µs | 341.015 µs–348.508 µs | 100 | 2.910 M elements/s |

### `apply` · criterion · all-features

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `core/send_end_to_end/1000_batches/1_events_each` | 194.487 µs | 192.919 µs–196.846 µs | 20 | 5.142 M elements/s |
| `core/send_end_to_end/100_batches/10_events_each` | 132.447 µs | 128.975 µs–132.986 µs | 20 | 7.550 M elements/s |
| `core/send_end_to_end/10_batches/100_events_each` | 120.750 µs | 120.082 µs–121.497 µs | 20 | 8.282 M elements/s |
| `core/send_end_to_end/1_batches/1000_events_each` | 129.400 µs | 128.891 µs–129.878 µs | 20 | 7.728 M elements/s |
| `core/send_topology/1_routers_10_workers/10_batches_1000_events_per_worker` | 368.061 µs | 367.402 µs–369.817 µs | 20 | 27.169 M elements/s |
| `core/send_topology/1_routers_1_workers/10_batches_1000_events_per_worker` | 129.789 µs | 127.595 µs–130.110 µs | 20 | 7.705 M elements/s |
| `core/send_topology/1_routers_2_workers/10_batches_1000_events_per_worker` | 143.776 µs | 143.215 µs–144.323 µs | 20 | 13.911 M elements/s |
| `core/send_topology/1_routers_4_workers/10_batches_1000_events_per_worker` | 188.481 µs | 187.619 µs–189.003 µs | 20 | 21.222 M elements/s |
| `core/send_topology/1_routers_8_workers/10_batches_1000_events_per_worker` | 322.535 µs | 321.462 µs–322.955 µs | 20 | 24.804 M elements/s |
| `core/send_topology/2_routers_10_workers/10_batches_1000_events_per_worker` | 375.682 µs | 373.809 µs–376.925 µs | 20 | 26.618 M elements/s |

### `apply` · criterion · default

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `core/send_end_to_end/1000_batches/1_events_each` | 191.546 µs | 191.111 µs–191.778 µs | 20 | 5.221 M elements/s |
| `core/send_end_to_end/100_batches/10_events_each` | 131.349 µs | 131.102 µs–131.951 µs | 20 | 7.613 M elements/s |
| `core/send_end_to_end/10_batches/100_events_each` | 121.942 µs | 121.742 µs–122.316 µs | 20 | 8.201 M elements/s |
| `core/send_end_to_end/1_batches/1000_events_each` | 129.070 µs | 129.000 µs–129.417 µs | 20 | 7.748 M elements/s |
| `core/send_topology/1_routers_10_workers/10_batches_1000_events_per_worker` | 368.433 µs | 365.988 µs–370.286 µs | 20 | 27.142 M elements/s |
| `core/send_topology/1_routers_1_workers/10_batches_1000_events_per_worker` | 130.716 µs | 130.469 µs–131.051 µs | 20 | 7.650 M elements/s |
| `core/send_topology/1_routers_2_workers/10_batches_1000_events_per_worker` | 141.463 µs | 141.290 µs–141.892 µs | 20 | 14.138 M elements/s |
| `core/send_topology/1_routers_4_workers/10_batches_1000_events_per_worker` | 188.337 µs | 187.149 µs–190.395 µs | 20 | 21.239 M elements/s |
| `core/send_topology/1_routers_8_workers/10_batches_1000_events_per_worker` | 320.111 µs | 319.622 µs–320.929 µs | 20 | 24.991 M elements/s |
| `core/send_topology/2_routers_10_workers/10_batches_1000_events_per_worker` | 375.422 µs | 374.249 µs–377.928 µs | 20 | 26.637 M elements/s |

### `listen` · criterion · all-features

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `listen/sustained_replay/1_routers_1_workers/baseline/1` | 19.796 ms | 15.393 ms–23.089 ms | 10 | 5.052 M elements/s |
| `listen/sustained_replay/1_routers_1_workers/baseline/100` | 19.324 ms | 16.745 ms–22.071 ms | 10 | 5.175 M elements/s |
| `listen/sustained_replay/1_routers_1_workers/baseline/1000` | 23.939 ms | 19.852 ms–27.403 ms | 10 | 4.177 M elements/s |
| `listen/sustained_replay/1_routers_1_workers/enabled/1` | 18.819 ms | 16.435 ms–23.348 ms | 10 | 5.314 M elements/s |
| `listen/sustained_replay/1_routers_1_workers/enabled/100` | 19.155 ms | 16.613 ms–22.549 ms | 10 | 5.221 M elements/s |
| `listen/sustained_replay/1_routers_1_workers/enabled/1000` | 23.026 ms | 19.970 ms–26.517 ms | 10 | 4.343 M elements/s |
| `listen/sustained_replay/2_routers_4_workers/baseline/1` | 19.149 ms | 16.267 ms–23.391 ms | 10 | 5.222 M elements/s |
| `listen/sustained_replay/2_routers_4_workers/baseline/100` | 7.711 ms | 6.052 ms–9.649 ms | 10 | 12.968 M elements/s |
| `listen/sustained_replay/2_routers_4_workers/baseline/1000` | 8.097 ms | 6.485 ms–10.039 ms | 10 | 12.351 M elements/s |
| `listen/sustained_replay/2_routers_4_workers/enabled/1` | 19.723 ms | 16.384 ms–23.390 ms | 10 | 5.070 M elements/s |
| `listen/sustained_replay/2_routers_4_workers/enabled/100` | 7.926 ms | 6.217 ms–10.046 ms | 10 | 12.617 M elements/s |
| `listen/sustained_replay/2_routers_4_workers/enabled/1000` | 8.181 ms | 6.653 ms–10.287 ms | 10 | 12.224 M elements/s |

### `listen` · criterion · default

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `listen/sustained_replay/1_routers_1_workers/baseline/1` | 18.592 ms | 15.246 ms–22.228 ms | 10 | 5.379 M elements/s |
| `listen/sustained_replay/1_routers_1_workers/baseline/100` | 19.547 ms | 16.101 ms–22.614 ms | 10 | 5.116 M elements/s |
| `listen/sustained_replay/1_routers_1_workers/baseline/1000` | 20.499 ms | 18.939 ms–22.682 ms | 10 | 4.878 M elements/s |
| `listen/sustained_replay/1_routers_1_workers/enabled/1` | 18.796 ms | 16.509 ms–22.558 ms | 10 | 5.320 M elements/s |
| `listen/sustained_replay/1_routers_1_workers/enabled/100` | 16.473 ms | 14.550 ms–19.357 ms | 10 | 6.070 M elements/s |
| `listen/sustained_replay/1_routers_1_workers/enabled/1000` | 23.064 ms | 20.095 ms–26.607 ms | 10 | 4.336 M elements/s |
| `listen/sustained_replay/2_routers_4_workers/baseline/1` | 19.541 ms | 16.328 ms–23.390 ms | 10 | 5.117 M elements/s |
| `listen/sustained_replay/2_routers_4_workers/baseline/100` | 7.370 ms | 5.855 ms–9.500 ms | 10 | 13.569 M elements/s |
| `listen/sustained_replay/2_routers_4_workers/baseline/1000` | 7.946 ms | 6.301 ms–9.512 ms | 10 | 12.585 M elements/s |
| `listen/sustained_replay/2_routers_4_workers/enabled/1` | 19.188 ms | 16.534 ms–22.877 ms | 10 | 5.212 M elements/s |
| `listen/sustained_replay/2_routers_4_workers/enabled/100` | 7.380 ms | 5.746 ms–9.159 ms | 10 | 13.550 M elements/s |
| `listen/sustained_replay/2_routers_4_workers/enabled/1000` | 8.154 ms | 6.579 ms–10.231 ms | 10 | 12.263 M elements/s |

### `monitor_ui` · criterion · all-features

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `terminal/in-memory-frame/160x40/1` | 106.999 µs | 106.929 µs–107.130 µs | 100 | — |
| `terminal/in-memory-frame/160x40/128` | 235.303 µs | 234.907 µs–235.754 µs | 100 | — |
| `terminal/in-memory-frame/160x40/4` | 119.316 µs | 119.234 µs–119.362 µs | 100 | — |
| `terminal/in-memory-frame/160x40/4096` | 236.698 µs | 236.447 µs–236.819 µs | 100 | — |
| `terminal/in-memory-frame/80x24/1` | 46.774 µs | 46.730 µs–46.825 µs | 100 | — |
| `terminal/in-memory-frame/80x24/128` | 64.027 µs | 63.991 µs–64.099 µs | 100 | — |
| `terminal/in-memory-frame/80x24/4` | 56.563 µs | 56.553 µs–56.593 µs | 100 | — |
| `terminal/in-memory-frame/80x24/4096` | 65.023 µs | 64.950 µs–65.067 µs | 100 | — |
| `terminal/scroll` | 1.003 ns | 1.001 ns–1.004 ns | 100 | — |
| `terminal/update-reading/1` | 58.501 ns | 58.474 ns–58.526 ns | 100 | — |
| `terminal/update-reading/128` | 921.911 ns | 921.275 ns–922.589 ns | 100 | — |
| `terminal/update-reading/4` | 79.952 ns | 79.840 ns–80.211 ns | 100 | — |
| `terminal/update-reading/4096` | 45.935 µs | 45.909 µs–45.956 µs | 100 | — |
| `terminal/waiting` | 19.114 µs | 19.103 µs–19.120 µs | 100 | — |

### `query` · criterion · all-features

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `core/query_end_to_end/1000_event_handles/1_routers_10_workers` | 22.219 µs | 22.064 µs–22.270 µs | 20 | 45.007 M elements/s |
| `core/query_end_to_end/1000_event_handles/1_routers_1_workers` | 22.156 µs | 22.130 µs–22.186 µs | 20 | 45.134 M elements/s |
| `core/query_end_to_end/1000_event_handles/1_routers_4_workers` | 22.350 µs | 22.304 µs–22.368 µs | 20 | 44.744 M elements/s |
| `core/query_end_to_end/1000_event_handles/2_routers_10_workers` | 23.626 µs | 23.598 µs–23.723 µs | 20 | 42.327 M elements/s |
| `core/query_end_to_end/1000_snapshots/1_routers_10_workers` | 77.167 µs | 76.937 µs–77.302 µs | 20 | 12.959 M elements/s |
| `core/query_end_to_end/1000_snapshots/1_routers_1_workers` | 66.233 µs | 66.155 µs–66.305 µs | 20 | 15.098 M elements/s |
| `core/query_end_to_end/1000_snapshots/1_routers_4_workers` | 47.732 µs | 39.065 µs–49.659 µs | 20 | 20.950 M elements/s |
| `core/query_end_to_end/1000_snapshots/2_routers_10_workers` | 77.947 µs | 77.782 µs–78.253 µs | 20 | 12.829 M elements/s |
| `core/query_end_to_end/100_event_handles/1_routers_10_workers` | 15.247 µs | 15.221 µs–15.263 µs | 20 | 6.559 M elements/s |
| `core/query_end_to_end/100_event_handles/1_routers_1_workers` | 15.577 µs | 15.539 µs–15.598 µs | 20 | 6.420 M elements/s |
| `core/query_end_to_end/100_event_handles/1_routers_4_workers` | 15.434 µs | 15.378 µs–15.556 µs | 20 | 6.479 M elements/s |
| `core/query_end_to_end/100_event_handles/2_routers_10_workers` | 16.945 µs | 16.928 µs–16.992 µs | 20 | 5.901 M elements/s |
| `core/query_end_to_end/100_snapshots/1_routers_10_workers` | 57.894 µs | 57.564 µs–58.001 µs | 20 | 1.727 M elements/s |
| `core/query_end_to_end/100_snapshots/1_routers_1_workers` | 22.389 µs | 22.376 µs–22.436 µs | 20 | 4.467 M elements/s |
| `core/query_end_to_end/100_snapshots/1_routers_4_workers` | 41.943 µs | 41.683 µs–42.478 µs | 20 | 2.384 M elements/s |
| `core/query_end_to_end/100_snapshots/2_routers_10_workers` | 63.971 µs | 63.746 µs–64.198 µs | 20 | 1.563 M elements/s |
| `core/query_end_to_end/1_event_handles/1_routers_10_workers` | 12.167 µs | 12.086 µs–12.227 µs | 20 | 82.189 k elements/s |
| `core/query_end_to_end/1_event_handles/1_routers_1_workers` | 12.292 µs | 12.237 µs–12.334 µs | 20 | 81.355 k elements/s |
| `core/query_end_to_end/1_event_handles/1_routers_4_workers` | 12.174 µs | 11.402 µs–12.230 µs | 20 | 82.141 k elements/s |
| `core/query_end_to_end/1_event_handles/2_routers_10_workers` | 10.738 µs | 10.636 µs–10.835 µs | 20 | 93.127 k elements/s |
| `core/query_end_to_end/1_snapshots/1_routers_10_workers` | 12.142 µs | 12.089 µs–12.179 µs | 20 | 82.358 k elements/s |
| `core/query_end_to_end/1_snapshots/1_routers_1_workers` | 12.194 µs | 12.166 µs–12.232 µs | 20 | 82.006 k elements/s |
| `core/query_end_to_end/1_snapshots/1_routers_4_workers` | 11.992 µs | 11.408 µs–12.105 µs | 20 | 83.392 k elements/s |
| `core/query_end_to_end/1_snapshots/2_routers_10_workers` | 11.258 µs | 11.092 µs–11.321 µs | 20 | 88.825 k elements/s |

### `query` · criterion · default

| Benchmark | Median | 95% interval | Samples | Declared-unit rate |
| --- | ---: | ---: | ---: | ---: |
| `core/query_end_to_end/1000_event_handles/1_routers_10_workers` | 22.238 µs | 22.229 µs–22.254 µs | 20 | 44.967 M elements/s |
| `core/query_end_to_end/1000_event_handles/1_routers_1_workers` | 22.165 µs | 22.148 µs–22.188 µs | 20 | 45.115 M elements/s |
| `core/query_end_to_end/1000_event_handles/1_routers_4_workers` | 22.293 µs | 22.265 µs–22.321 µs | 20 | 44.857 M elements/s |
| `core/query_end_to_end/1000_event_handles/2_routers_10_workers` | 23.268 µs | 23.215 µs–23.292 µs | 20 | 42.977 M elements/s |
| `core/query_end_to_end/1000_snapshots/1_routers_10_workers` | 78.222 µs | 77.413 µs–78.808 µs | 20 | 12.784 M elements/s |
| `core/query_end_to_end/1000_snapshots/1_routers_1_workers` | 65.795 µs | 65.695 µs–65.950 µs | 20 | 15.199 M elements/s |
| `core/query_end_to_end/1000_snapshots/1_routers_4_workers` | 57.013 µs | 54.049 µs–57.284 µs | 20 | 17.540 M elements/s |
| `core/query_end_to_end/1000_snapshots/2_routers_10_workers` | 78.669 µs | 78.442 µs–79.183 µs | 20 | 12.711 M elements/s |
| `core/query_end_to_end/100_event_handles/1_routers_10_workers` | 15.522 µs | 15.478 µs–15.588 µs | 20 | 6.442 M elements/s |
| `core/query_end_to_end/100_event_handles/1_routers_1_workers` | 15.362 µs | 15.336 µs–15.382 µs | 20 | 6.510 M elements/s |
| `core/query_end_to_end/100_event_handles/1_routers_4_workers` | 15.726 µs | 15.525 µs–16.005 µs | 20 | 6.359 M elements/s |
| `core/query_end_to_end/100_event_handles/2_routers_10_workers` | 16.727 µs | 16.686 µs–16.753 µs | 20 | 5.978 M elements/s |
| `core/query_end_to_end/100_snapshots/1_routers_10_workers` | 57.883 µs | 57.566 µs–58.133 µs | 20 | 1.728 M elements/s |
| `core/query_end_to_end/100_snapshots/1_routers_1_workers` | 22.570 µs | 22.546 µs–22.625 µs | 20 | 4.431 M elements/s |
| `core/query_end_to_end/100_snapshots/1_routers_4_workers` | 41.057 µs | 36.334 µs–42.356 µs | 20 | 2.436 M elements/s |
| `core/query_end_to_end/100_snapshots/2_routers_10_workers` | 63.747 µs | 63.523 µs–64.023 µs | 20 | 1.569 M elements/s |
| `core/query_end_to_end/1_event_handles/1_routers_10_workers` | 11.559 µs | 10.908 µs–12.226 µs | 20 | 86.514 k elements/s |
| `core/query_end_to_end/1_event_handles/1_routers_1_workers` | 12.283 µs | 11.958 µs–12.321 µs | 20 | 81.413 k elements/s |
| `core/query_end_to_end/1_event_handles/1_routers_4_workers` | 11.965 µs | 11.509 µs–12.128 µs | 20 | 83.574 k elements/s |
| `core/query_end_to_end/1_event_handles/2_routers_10_workers` | 11.402 µs | 11.287 µs–11.591 µs | 20 | 87.701 k elements/s |
| `core/query_end_to_end/1_snapshots/1_routers_10_workers` | 12.129 µs | 11.646 µs–12.187 µs | 20 | 82.445 k elements/s |
| `core/query_end_to_end/1_snapshots/1_routers_1_workers` | 12.239 µs | 12.207 µs–12.299 µs | 20 | 81.703 k elements/s |
| `core/query_end_to_end/1_snapshots/1_routers_4_workers` | 12.199 µs | 12.131 µs–12.228 µs | 20 | 81.975 k elements/s |
| `core/query_end_to_end/1_snapshots/2_routers_10_workers` | 12.394 µs | 12.343 µs–12.455 µs | 20 | 80.685 k elements/s |
