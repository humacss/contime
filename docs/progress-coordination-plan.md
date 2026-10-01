# Reusable progress coordination

Branch: `codex/reusable-progress-coordination`, based on `269b176`.
The original checkout and its uncommitted formatting change are untouched.

## Passes

1. Extract the measurement-round state machine into `contime-progress`.
   Keep admission, routing fences, worker execution and pruning policy in Core.
   Reuse the extracted implementation from the existing pruning frontier.
   Verify the existing Core regressions and add isolated protocol tests.
2. Separate worker round resolution from pruning and introduce a bounded,
   injected observation policy. Share serialized measurement rounds; do not run
   competing coordinators against the same workers. Preserve admission rules.
3. Pass the injection through Runtime, then connect Engine/Stream measurement.
   Stream remains responsible for distributed membership and in-flight work.

The first pass does not provide a complete distributed safety protocol or an
Engine progress API. Measurements are exclusive boundaries. Policy owns any
monotonic committed frontier; a measurement alone does not forbid later input.

## Verification

- Establish the unmodified `contime-core` test baseline.
- Test the extracted public API before implementation: capping, pending work,
  stale/duplicate reports, admissions during a round, missing participants,
  complete timestamps, and reuse with different policies.
- Run all `contime-progress` and `contime-core` tests after extraction.
- Preserve the existing controlled replay-frontier and threaded feedback tests.

## Status

- Isolation: complete.
- Baseline: complete, 79 Core tests passed; nine inline benchmarks ignored.
- Extraction: complete; seven public-API protocol tests and all 79 Core tests
  pass. Nine inline Core benchmarks remain ignored. Strict Clippy passes for
  the new crate. Performance has not yet been remeasured.
- Observer injection: implemented in Core through `start_with_progress` and a
  `ProgressPolicy` in the existing coordinator thread. Shared rounds retain
  separate observation/pruning cutoffs. An unchanged prune boundary resumes
  workers through the existing no-op resolution; worker/router APIs need no
  change. Full verification passes: 87 Core tests and seven progress protocol
  tests; nine inline Core benchmarks remain ignored. Strict Clippy passes for
  the Core library and progress integration test, and all progress-crate
  targets. Performance has not yet been remeasured.
- Downstream Runtime/Engine startup injection: implemented in isolated sibling
  worktrees under `/private/tmp/progress-coordination/timeless-4d-games` on the
  same branch name. Runtime preserves complete timestamps and host context;
  Engine recreates its injected policy when seek replaces Runtime. New tests
  confirm local completion is not dispatch completion. Runtime root and Engine
  root/macro suites pass, including Engine's separate-process TCP test.
- Stream group/session and in-flight accounting: pending. Local reports alone
  must not govern distributed advancement.
