# Native scaling and latency benchmarks

This standalone, unpublished benchmark package measures native worker scaling
and end-to-end application/query latency. It is separate from the browser
example, and is excluded from the workspace and published packages.

Run commands from the ConTime repository root:

```sh
cargo +1.90.0 test --manifest-path benches/scaling/Cargo.toml
cargo +1.90.0 clippy --manifest-path benches/scaling/Cargo.toml --all-targets -- -D warnings
cargo +1.90.0 build --release --manifest-path benches/scaling/Cargo.toml --bins

# Throughput: events, arithmetic rounds/event, events/submission, samples,
# execution layers, comma-separated response-channel pool sizes.
benches/scaling/target/release/contime-scaling-bench 524288 0 1 5 full 1,64
benches/scaling/target/release/contime-scaling-bench 524288 0 1000 5 full 1,64

# Latency: events, workers, events/submission, arrival pattern, query interval (us).
benches/scaling/target/release/latency 65536 4 1 burst 2000
```

The scaling layers are direct `compute`, preloaded real workers (`preloaded`),
streaming real workers (`workers`), router plus workers (`routed`), and the public
API (`full`). Fixed total work is distributed over 64 histories and one, two,
and four workers. Event counts must be positive multiples of 64. Each run checks
counts and order-sensitive canonical fingerprints, including with zero extra
arithmetic rounds. Response channels rotate by submission index, independently
of event IDs or batch alignment. Event records and submission vectors are built
before timing. Streaming timing includes routing/conversion allocation and
submission; final public verification and shutdown are outside timing.

The CPU-time probes support macOS and Linux. Threads are unpinned.

Latency patterns are `saturated` (unpaced), `burst` (1,024 events every 5 ms),
and `sparse` (one event per millisecond). Submissions are split at arrival-group
boundaries, so caller batch size never changes the offered arrival schedule.
Late producer wakeups catch up against the original schedule without adding drift. The probe records live
application and concurrent query latency; independent final-state verification
is outside timing. Startup idle helpers are used only as benchmark controls.

Run one benchmark at a time on an otherwise idle host. Compare matching workload,
compiler, dependency versions, and build profiles. These measure the full native
pipeline; use `cargo bench -p contime-batcher --bench batcher` to isolate batching.

Compare revisions with identical probe sources, compiler, dependencies, and build
profiles. The zero-round workload still uses an order-sensitive hash. Report
medians and spread rather than a single run.

The latency probe queries the final timestamp while processing catches up.
Frequent queries can repeatedly reconstruct unprocessed event tails, so this is
a query-stress workload. Its completion rate is not baseline application throughput.

## Recorded results

A matched 2026-10-28 comparison of pre-batching `49b7f61` and candidate `7d436d6`
used these corrected probes, Rust 1.90.0, identical external dependency versions,
and alternating samples on an unpinned M3 Pro under background load. The baseline
also received the published Traitstub compatibility patch (`5d11822`). For 524,288
events over 64 snapshot IDs, five-sample medians with one shared response channel:

| Workers | Individual submissions | Batches of 1,000 |
| --- | --- | --- |
| 1 | 3.95 M events/s | 4.03 M events/s |
| 2 | 6.93 M events/s | 7.97 M events/s |
| 4 | 8.49 M events/s | 17.88 M events/s |

The candidate improves four-worker individual-submission completion time by about
2×; batched medians are within about 2% of the baseline. Both revisions already
contain the replay scheduling fix. These finite-work results describe this machine
and workload, not a sustained performance guarantee.

The latency probe queries the final timestamp while processing is still catching
up. Frequent queries can repeatedly reconstruct unprocessed event tails; at a 2 ms
query interval this is a query-stress benchmark. In a one-worker control, the
candidate completed 262,144 individual submissions in a median 74 ms without
periodic queries, 75 ms with a 50 ms interval, and 3.8 s with a 2 ms interval.
Those short controls recorded zero, one, and roughly 1,200 queries respectively.
Do not interpret the query-stress completion rate as baseline application throughput.

## Full probe run — 2026-10-28

Apple M3 Pro, 11 logical CPUs, Rust 1.90.0, optimized release build, unpinned
threads, normal desktop background load. These probes ran sequentially after the
workspace benchmarks, using the default Runtime build without optional counters.
The isolated probe resolved Crossbeam channel 0.5.17 and Crossbeam utils 0.8.23.
Every run verified counts and the canonical order-sensitive hash. Initial record
preparation, final verification, and shutdown are excluded from throughput timing.

### Fixed-work throughput

524,288 events, 64 snapshot IDs, zero extra arithmetic rounds, five samples per
configuration. Wall-time medians include the observed min–max range. Other
columns are medians; throughput is event count divided by median wall time.
`compute` measures arithmetic alone; the other modes include progressively more
of the real worker/router/public pipeline. CPU times sum the applicable threads,
so they can exceed wall time. Worker CPU/span cover the probe’s recorded
processing interval, not entire thread lifetimes. `preloaded` excludes initial
worker-queue loading. The response-pool setting is unused in `compute` mode.

| Layer | Submission | Response pool | Workers | Wall ms (min–max) | M events/s | Process CPU ms | Submit ms | Worker CPU ms | Max worker span ms | Application calls |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| compute | 1 | 1 | 1 | 2.231 (1.128–3.811) | 235.001 | 2.253 | 0.003 | 2.197 | 2.197 | 0 |
| compute | 1 | 1 | 2 | 1.850 (0.777–1.975) | 283.399 | 3.511 | 0.003 | 3.436 | 1.830 | 0 |
| compute | 1 | 1 | 4 | 0.746 (0.399–1.209) | 702.799 | 2.197 | 0.004 | 2.076 | 0.726 | 0 |
| compute | 1 | 64 | 1 | 2.339 (1.644–2.793) | 224.150 | 2.352 | 0.003 | 2.314 | 2.314 | 0 |
| compute | 1 | 64 | 2 | 1.302 (0.620–1.538) | 402.679 | 2.381 | 0.004 | 2.304 | 1.284 | 0 |
| compute | 1 | 64 | 4 | 0.835 (0.318–1.382) | 627.890 | 3.329 | 0.005 | 3.252 | 0.818 | 0 |
| compute | 1,000 | 1 | 1 | 2.620 (0.547–5.254) | 200.110 | 2.634 | 0.003 | 2.596 | 2.596 | 0 |
| compute | 1,000 | 1 | 2 | 0.640 (0.550–1.328) | 819.200 | 1.172 | 0.003 | 1.115 | 0.623 | 0 |
| compute | 1,000 | 1 | 4 | 0.803 (0.441–1.355) | 652.912 | 2.430 | 0.005 | 2.332 | 0.787 | 0 |
| compute | 1,000 | 64 | 1 | 2.334 (0.529–3.925) | 224.631 | 2.339 | 0.002 | 2.309 | 2.309 | 0 |
| compute | 1,000 | 64 | 2 | 1.032 (0.562–1.775) | 508.031 | 1.939 | 0.002 | 1.827 | 1.014 | 0 |
| compute | 1,000 | 64 | 4 | 0.778 (0.407–2.004) | 673.892 | 2.800 | 0.004 | 2.701 | 0.760 | 0 |
| full | 1 | 1 | 1 | 129.372 (128.790–186.293) | 4.053 | 251.008 | 56.864 | 129.250 | 129.352 | 524,288 |
| full | 1 | 1 | 2 | 76.191 (74.176–102.126) | 6.881 | 287.951 | 63.441 | 150.980 | 76.164 | 524,288 |
| full | 1 | 1 | 4 | 62.186 (62.063–64.077) | 8.431 | 311.334 | 58.446 | 175.268 | 62.153 | 524,288 |
| full | 1 | 64 | 1 | 148.525 (145.559–150.231) | 3.530 | 296.364 | 49.890 | 148.283 | 148.414 | 524,288 |
| full | 1 | 64 | 2 | 83.662 (82.089–102.970) | 6.267 | 325.766 | 50.242 | 165.924 | 83.526 | 524,288 |
| full | 1 | 64 | 4 | 72.361 (71.163–101.972) | 7.245 | 419.683 | 45.121 | 260.786 | 72.338 | 524,288 |
| full | 1,000 | 1 | 1 | 127.735 (125.968–131.360) | 4.104 | 144.557 | 7.787 | 127.626 | 127.690 | 524,288 |
| full | 1,000 | 1 | 2 | 66.208 (64.560–107.859) | 7.919 | 148.296 | 8.069 | 131.573 | 66.155 | 524,288 |
| full | 1,000 | 1 | 4 | 29.409 (28.912–34.909) | 17.827 | 134.681 | 7.324 | 115.800 | 29.360 | 524,288 |
| full | 1,000 | 64 | 1 | 127.400 (125.024–133.898) | 4.115 | 144.066 | 8.067 | 127.350 | 127.350 | 524,288 |
| full | 1,000 | 64 | 2 | 65.250 (64.628–68.265) | 8.035 | 147.152 | 8.148 | 129.978 | 65.195 | 524,288 |
| full | 1,000 | 64 | 4 | 29.512 (29.088–51.404) | 17.765 | 134.544 | 7.357 | 115.729 | 29.466 | 524,288 |
| preloaded | 1 | 1 | 1 | 85.894 (85.026–124.363) | 6.104 | 86.258 | 0.003 | 85.865 | 85.865 | 524,288 |
| preloaded | 1 | 1 | 2 | 49.636 (46.708–61.795) | 10.563 | 99.208 | 0.004 | 98.971 | 49.603 | 524,288 |
| preloaded | 1 | 1 | 4 | 27.392 (26.511–70.419) | 19.140 | 109.241 | 0.005 | 108.935 | 27.368 | 524,288 |
| preloaded | 1 | 64 | 1 | 87.031 (85.097–116.069) | 6.024 | 87.380 | 0.004 | 87.005 | 87.014 | 524,288 |
| preloaded | 1 | 64 | 2 | 43.790 (43.289–55.158) | 11.973 | 87.702 | 0.004 | 87.350 | 43.758 | 524,288 |
| preloaded | 1 | 64 | 4 | 23.703 (21.953–40.961) | 22.119 | 94.514 | 0.004 | 94.167 | 23.677 | 524,288 |
| preloaded | 1,000 | 1 | 1 | 69.537 (63.048–85.754) | 7.540 | 68.799 | 0.003 | 68.399 | 69.517 | 524,288 |
| preloaded | 1,000 | 1 | 2 | 35.146 (31.243–36.108) | 14.917 | 68.700 | 0.004 | 68.331 | 35.123 | 524,288 |
| preloaded | 1,000 | 1 | 4 | 15.814 (14.942–19.751) | 33.153 | 62.827 | 0.005 | 62.489 | 15.789 | 524,288 |
| preloaded | 1,000 | 64 | 1 | 65.417 (63.240–86.672) | 8.015 | 65.681 | 0.003 | 65.243 | 65.399 | 524,288 |
| preloaded | 1,000 | 64 | 2 | 32.691 (31.002–38.085) | 16.038 | 65.247 | 0.004 | 64.897 | 32.663 | 524,288 |
| preloaded | 1,000 | 64 | 4 | 15.812 (15.215–16.661) | 33.158 | 62.401 | 0.005 | 62.027 | 15.788 | 524,288 |
| routed | 1 | 1 | 1 | 161.862 (157.130–164.862) | 3.239 | 264.903 | 45.048 | 161.752 | 161.760 | 524,288 |
| routed | 1 | 1 | 2 | 96.897 (96.435–101.469) | 5.411 | 312.757 | 39.441 | 192.941 | 96.876 | 524,288 |
| routed | 1 | 1 | 4 | 103.537 (102.561–154.024) | 5.064 | 559.164 | 47.886 | 410.008 | 103.516 | 524,288 |
| routed | 1 | 64 | 1 | 156.830 (154.925–161.777) | 3.343 | 249.339 | 35.715 | 156.725 | 156.779 | 524,288 |
| routed | 1 | 64 | 2 | 91.851 (91.059–107.019) | 5.708 | 294.178 | 30.898 | 183.169 | 91.831 | 524,288 |
| routed | 1 | 64 | 4 | 101.670 (98.633–115.782) | 5.157 | 534.927 | 30.499 | 402.902 | 101.652 | 524,288 |
| routed | 1,000 | 1 | 1 | 127.436 (124.893–162.770) | 4.114 | 142.398 | 7.661 | 127.366 | 127.393 | 524,288 |
| routed | 1,000 | 1 | 2 | 63.801 (63.006–69.384) | 8.218 | 139.056 | 6.839 | 127.196 | 63.750 | 524,288 |
| routed | 1,000 | 1 | 4 | 31.349 (29.525–35.063) | 16.724 | 133.188 | 7.210 | 119.877 | 31.268 | 524,288 |
| routed | 1,000 | 64 | 1 | 132.874 (125.453–154.404) | 3.946 | 146.530 | 7.642 | 132.574 | 132.839 | 524,288 |
| routed | 1,000 | 64 | 2 | 66.482 (63.305–66.948) | 7.886 | 142.788 | 7.117 | 131.123 | 66.448 | 524,288 |
| routed | 1,000 | 64 | 4 | 30.442 (29.133–42.896) | 17.223 | 131.701 | 7.109 | 118.062 | 30.395 | 524,288 |
| workers | 1 | 1 | 1 | 154.259 (151.974–176.722) | 3.399 | 205.485 | 51.323 | 154.136 | 154.158 | 524,288 |
| workers | 1 | 1 | 2 | 98.892 (95.954–103.602) | 5.302 | 283.236 | 86.059 | 196.894 | 98.830 | 524,288 |
| workers | 1 | 1 | 4 | 103.854 (98.440–144.871) | 5.048 | 515.536 | 103.849 | 411.546 | 103.821 | 524,288 |
| workers | 1 | 64 | 1 | 151.496 (150.964–157.702) | 3.461 | 196.331 | 45.040 | 151.448 | 151.455 | 524,288 |
| workers | 1 | 64 | 2 | 94.282 (91.262–95.486) | 5.561 | 259.374 | 72.406 | 187.279 | 94.254 | 524,288 |
| workers | 1 | 64 | 4 | 90.666 (89.045–133.486) | 5.783 | 450.502 | 90.661 | 359.855 | 90.629 | 524,288 |
| workers | 1,000 | 1 | 1 | 126.806 (125.617–292.692) | 4.135 | 135.726 | 7.557 | 126.733 | 126.740 | 524,288 |
| workers | 1,000 | 1 | 2 | 66.322 (63.075–67.476) | 7.905 | 138.175 | 7.612 | 130.506 | 66.261 | 524,288 |
| workers | 1,000 | 1 | 4 | 30.397 (28.786–37.437) | 17.248 | 125.946 | 8.168 | 117.739 | 30.352 | 524,288 |
| workers | 1,000 | 64 | 1 | 125.938 (125.094–179.761) | 4.163 | 132.932 | 6.995 | 125.881 | 125.881 | 524,288 |
| workers | 1,000 | 64 | 2 | 65.031 (63.564–69.062) | 8.062 | 136.482 | 7.771 | 128.695 | 64.975 | 524,288 |
| workers | 1,000 | 64 | 4 | 29.238 (28.970–37.427) | 17.932 | 123.355 | 7.850 | 115.471 | 29.188 | 524,288 |

### Application and concurrent-query latency

Three samples per configuration. Saturated runs submit 262,144 events; burst runs
submit 65,536 at 1,024 per 5 ms; sparse runs submit 640 at one event/ms. All query
sweeps target the final timestamp at a 2 ms interval. Percentile columns report
the median of each run's p50/p95/p99, not percentiles pooled across runs. The
completion rate includes per-event timing and concurrent-query overhead. Frequent
queries ahead of live progress can repeatedly reconstruct the unprocessed tail;
these rates are not comparable with query-free throughput above.

| Pattern | Submission | Workers | Elapsed ms | Events/s | Application p50 / p95 / p99 µs | Query p50 / p95 / p99 µs | Query count |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| burst | 1 | 1 | 317.162 | 206,633 | 216.250 / 636.541 / 1,529.083 | 71.542 / 151.792 / 324.709 | 127 |
| burst | 1 | 4 | 315.976 | 207,408 | 147.167 / 597.333 / 785.458 | 79.834 / 283.667 / 498.500 | 130 |
| burst | 1,000 | 1 | 316.519 | 207,052 | 493.500 / 1,232.917 / 2,771.541 | 108.083 / 334.125 / 788.000 | 128 |
| burst | 1,000 | 4 | 316.479 | 207,079 | 213.833 / 431.417 / 847.167 | 78.417 / 162.334 / 344.792 | 128 |
| saturated | 1 | 1 | 4,417.271 | 59,345 | 4,078,157.583 / 4,368,039.083 / 4,368,666.458 | 3,208.291 / 5,851.208 / 10,635.541 | 1,260 |
| saturated | 1 | 4 | 49.803 | 5,263,605 | 102.709 / 383.334 / 671.583 | 94.959 / 652.542 / 652.542 | 20 |
| saturated | 1,000 | 1 | 4,276.683 | 61,296 | 3,994,448.209 / 4,269,461.708 / 4,271,012.208 | 3,282.625 / 5,202.584 / 11,683.500 | 1,226 |
| saturated | 1,000 | 4 | 25.303 | 10,360,041 | 17,077.000 / 19,675.541 / 19,928.625 | 1,175.833 / 4,310.833 / 4,310.833 | 10 |
| sparse | 1 | 1 | 639.315 | 1,001 | 43.667 / 418.166 / 3,326.333 | 56.292 / 241.958 / 1,836.750 | 257 |
| sparse | 1 | 4 | 639.253 | 1,001 | 29.375 / 94.042 / 213.667 | 61.041 / 154.959 / 245.333 | 263 |
| sparse | 1,000 | 1 | 639.289 | 1,001 | 29.958 / 100.958 / 172.625 | 44.709 / 158.334 / 284.542 | 261 |
| sparse | 1,000 | 4 | 639.296 | 1,001 | 31.833 / 77.959 / 143.542 | 62.416 / 143.875 / 438.000 | 263 |
