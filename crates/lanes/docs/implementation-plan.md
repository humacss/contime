# Isolated Lanes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build an isolated Rust crate for zero-cost raw, filter, and apply lane dispatch.

**Architecture:** Generic associated batch types preserve borrowed, allocation-free default projection. A stateless filter emits an apply-lane batch through static dispatch, and routing initializes a default snapshot independently of filtering.

**Tech Stack:** Rust 2021 and the standard library.

**Spec:** `docs/design.md`

## Global Constraints

- Do not depend on ConTime or any sibling subcrate.
- Keep all tests inline; add no integration-test directory.
- Keep `lib.rs` limited to modules and public re-exports.
- Use static dispatch and no trait objects.

---

### Task 1: Routing and initialization

**Files:** `src/types.rs`, `src/route.rs`

**Interfaces:** `Route<S>::snapshot_id`, `Route<S>::initialize_snapshot`, and `Route<S>::create_snapshot`.

- [ ] Write an inline test proving creation preserves `S::default()` time while initializing identity.
- [ ] Run `cargo test --manifest-path crates/lanes/Cargo.toml route::tests` and observe the missing contract failure.
- [ ] Add the minimal route contract and implementation.
- [ ] Re-run the focused test and observe it pass.

### Task 2: Projection, filtering, and apply delivery

**Files:** `src/types.rs`, `src/filter.rs`, `src/apply.rs`

**Interfaces:** `Lanes`, `FilterLanes<R>`, `FilterBatch`, `ApplyBatch`, `EventFilter`, `FilterOutput`, `PassThrough`, `filter`, `ApplyEvents`, and `apply`.

- [ ] Write inline tests proving raw projection preserves order and references.
- [ ] Run the focused filter tests and observe the missing behavior failure.
- [ ] Implement the minimal projection contracts and pass-through filter.
- [ ] Write an inline apply test whose filter emits a decorated apply-only event.
- [ ] Run the focused apply test and observe the missing behavior failure.
- [ ] Implement static apply delivery using the filter output type.
- [ ] Re-run all crate unit tests and observe them pass.

### Task 3: Verification

**Files:** all crate Rust sources.

- [ ] Run `cargo fmt --manifest-path crates/lanes/Cargo.toml -- --check`.
- [ ] Run `cargo test --manifest-path crates/lanes/Cargo.toml`.
- [ ] Run `cargo check --manifest-path crates/lanes/Cargo.toml`.
