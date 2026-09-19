# ConTime performance comparison — 2026-09-19

Compared 46 fresh optimized Criterion measurements against the historical tables in `crates/core/README.md` (2026-08-31 and 2026-09-01). All three benchmark executables completed successfully. No implementation changes were made for this measurement.

## Method and limitations

- Toolchain: rustc 1.95.0, aarch64-apple-darwin; optimized bench profile.
- Each case: 1-second warm-up, 3-second requested measurement, 20 samples. Slow cases can exceed the requested duration.
- Baseline: previously recorded numbers, not an old revision rebuilt on this run. Differences include any intervening compiler/dependency/machine-condition changes. These are historical comparisons, not controlled attribution to this patch.
- Query figures use Criterion means, matching the old query table. Other figures use Criterion's displayed estimate (slope where available, mean for flat sampling).
- Processing benchmarks include completion via the new idle wait; they do not merely time asynchronous enqueue.
- Advance benchmarks include completion of safe pruning, not merely dispatch of the advance message.
- These are wall-clock latency measurements, not CPU consumption.
- No client, reactor-feedback workload, listener timing suite or unit timing suite was measured. In particular, these results do not establish whether feedback amplification has improved.
- Raw data: `crates/core/target/criterion/**/incremental-20260919/`.

Command:

```sh
cargo +1.95.0 bench --offline --manifest-path crates/core/Cargo.toml \
  --bench apply --bench query --bench advance -- \
  --noplot --warm-up-time 1 --measurement-time 3 --sample-size 20 \
  --save-baseline incremental-20260919
```

## Main findings

- One-event submissions improved: 754.080 → 339.013 µs for 1,000 events (55% lower latency).
- Larger batches regressed: ten batches of 100 events rose from 84.606 → 253.832 µs (3.00× latency).
- Multi-worker event throughput also regressed: two routers/ten workers, 10,000 events, 311.410 → 622.549 µs (2.00× latency).
- Single-result queries are about 31–38% lower latency; larger query improvements vary, with some essentially unchanged.
- Clean pruning improved across measured topologies. Anchor materialization improved with one/four workers but regressed with ten workers.
- Pruning with unfinished replay now averages 71–105 ms versus 0.880–1.857 ms historically. The coordinator explicitly limits measurement rounds to one per 100 ms; when the first report sees pending work, completing safe pruning waits for another round. The measured delay is consistent with that mechanism, not evidence of 100 ms of CPU work. The benchmark waits for full pruning; ordinary advance submission remains asynchronous.
- Batch-processing regressions need profiling before assigning a cause. Incremental scheduling, admission coordination and completion-observation overhead are candidates, not measured causal breakdowns.

## Full comparison

All times below are µs. Negative change means lower latency.

### Event processing — 1,000 total events, one router and worker

| Case | Old µs | New µs | Latency change |
| --- | ---: | ---: | ---: |
| 1000 batches/1 events each | 754.080 | 339.013 | -55.0% |
| 100 batches/10 events each | 143.760 | 254.176 | +76.8% |
| 10 batches/100 events each | 84.606 | 253.832 | +200.0% |
| 1 batches/1000 events each | 94.436 | 266.894 | +182.6% |

### Event processing — 1,000 events per worker, ten batches

| Case | Old µs | New µs | Latency change |
| --- | ---: | ---: | ---: |
| 1r/10w/10 batches 1000 events/worker | 305.730 | 556.441 | +82.0% |
| 1r/1w/10 batches 1000 events/worker | 77.523 | 247.739 | +219.6% |
| 1r/2w/10 batches 1000 events/worker | 114.820 | 264.117 | +130.0% |
| 1r/4w/10 batches 1000 events/worker | 162.060 | 308.796 | +90.5% |
| 1r/8w/10 batches 1000 events/worker | 275.610 | 491.043 | +78.2% |
| 2r/10w/10 batches 1000 events/worker | 311.410 | 622.549 | +99.9% |

### Read-only queries

| Case | Old µs | New µs | Latency change |
| --- | ---: | ---: | ---: |
| 1000 event handles/1r/10w | 32.224 | 25.388 | -21.2% |
| 1000 event handles/1r/1w | 31.679 | 25.101 | -20.8% |
| 1000 event handles/1r/4w | 31.912 | 25.208 | -21.0% |
| 1000 event handles/2r/10w | 30.556 | 27.165 | -11.1% |
| 1000 snapshots/1r/10w | 96.442 | 78.145 | -19.0% |
| 1000 snapshots/1r/1w | 65.376 | 59.306 | -9.3% |
| 1000 snapshots/1r/4w | 67.559 | 57.095 | -15.5% |
| 1000 snapshots/2r/10w | 96.037 | 81.596 | -15.0% |
| 100 event handles/1r/10w | 22.142 | 15.230 | -31.2% |
| 100 event handles/1r/1w | 22.110 | 15.170 | -31.4% |
| 100 event handles/1r/4w | 21.704 | 15.195 | -30.0% |
| 100 event handles/2r/10w | 21.619 | 16.850 | -22.1% |
| 100 snapshots/1r/10w | 67.317 | 61.557 | -8.6% |
| 100 snapshots/1r/1w | 27.238 | 21.096 | -22.5% |
| 100 snapshots/1r/4w | 45.749 | 45.480 | -0.6% |
| 100 snapshots/2r/10w | 64.582 | 63.590 | -1.5% |
| 1 event handles/1r/10w | 18.435 | 12.351 | -33.0% |
| 1 event handles/1r/1w | 17.900 | 12.392 | -30.8% |
| 1 event handles/1r/4w | 19.736 | 12.441 | -37.0% |
| 1 event handles/2r/10w | 17.462 | 11.543 | -33.9% |
| 1 snapshots/1r/10w | 19.438 | 12.538 | -35.5% |
| 1 snapshots/1r/1w | 19.101 | 12.626 | -33.9% |
| 1 snapshots/1r/4w | 20.323 | 12.613 | -37.9% |
| 1 snapshots/2r/10w | 19.022 | 12.508 | -34.2% |

### Advance through completed pruning — 1,000 histories

| Case | Old µs | New µs | Latency change |
| --- | ---: | ---: | ---: |
| anchor/1r 10w | 891.700 | 1527.401 | +71.3% |
| anchor/1r 1w | 664.200 | 134.823 | -79.7% |
| anchor/1r 4w | 426.700 | 300.336 | -29.6% |
| anchor/2r 10w | 406.900 | 1519.736 | +273.5% |
| clean/1r 10w | 1202.000 | 900.369 | -25.1% |
| clean/1r 1w | 328.900 | 87.183 | -73.5% |
| clean/1r 4w | 291.000 | 266.313 | -8.5% |
| clean/2r 10w | 1068.000 | 737.287 | -31.0% |
| dirty/1r 10w | 1857.000 | 91961.393 | +4852.1% |
| dirty/1r 1w | 1471.000 | 105230.454 | +7053.7% |
| dirty/1r 4w | 1628.000 | 104472.827 | +6317.2% |
| dirty/2r 10w | 879.800 | 71359.434 | +8010.9% |

