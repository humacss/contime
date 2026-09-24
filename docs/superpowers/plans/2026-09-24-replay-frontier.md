# Replay frontier implementation plan

Goal: prevent safe-frontier advancement from overtaking possible replay publications.
Spec: approved design in the current conversation; forwarding remains effect-free.

- [x] Add deterministic regressions for report/resolution barriers and admissions during measurement; run red.
- [x] Expose earliest replay time from snapshot storage, using the same checkpoint selection as playback. Core adapts it to the worker trait.
- [x] Report storage-derived minima for pending histories. Pause live replay after reporting; resolve each round explicitly, forwarding before resumption.
- [x] Route round resolutions to every worker. Serialize rounds until resolution completion; admissions during a round prevent advancement.
- [x] Verify zero-progress release, multi-worker/multi-router delivery, inclusive horizon admission, eventual forwarding, and silent forwarding.
- [x] Run focused snapshots, worker, router and Core tests; update interface documentation. Do not commit unrelated work.

Implementation stays in the current checkout to preserve the uncommitted regression setup.
Tests use real component steps, not sleeps or timing retries. No change to apply callbacks,
query publication rules, or admission rejection semantics.

Verification uses `cargo +1.95.0 test --quiet --offline --locked --manifest-path
crates/<crate>/Cargo.toml --lib --tests --target-dir
/Users/johannaeslund/projects/github/timeless-4d-games/arcanex-client/target`
for core, worker, router, and snapshots. Inline benchmarks remain ignored.
Worker all-target compilation and touched-file rustfmt checks also pass.

Additional regression-driven corrections: consecutive rounds allow a processing
step before reporting again; completed horizon buckets do not pin future-only
history; completed scheduling cursors are not mistaken for unapplied events.
An independent read-only concurrency review found no actionable issues.
Continuous admission can delay pruning because a measurement containing new
admissions conservatively retains the old safe boundary. The game has not been rerun.
