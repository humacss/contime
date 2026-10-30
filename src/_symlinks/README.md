# Shared source links

The leading underscore marks this directory as internal crate assembly.

Why are there symbolic links under `src/`? ConTime keeps components as ordinary
independent development crates under the top-level `crates/`, but publishes their
implementation together as one public `contime` package.

Cargo excludes nested packages from the parent archive, even when their source
is listed in `include`. This directory exposes the same source without the
component manifests. Cargo follows these links and writes their contents into
the archive as ordinary files. No release script or source generation is needed.

```text
crates/worker/
├── Cargo.toml                     # Independent development package
├── README.md                     # Component documentation
└── src/
    ├── lib.rs                     # Standalone entry point
    ├── mod.rs                     # Embedded entry point
    └── ...                        # Actual implementation and unit tests

src/_symlinks/
├── README.md                     # This explanation
└── worker/
    └── src → ../../../crates/worker/src
```

## Where to edit

Edit implementation files under `crates/`. There is one maintained set of source
files, accessible through two paths. The root facade in `src/lib.rs` loads the
embedded `mod.rs` entry points through this directory. Standalone components use
`lib.rs`. Both entry points stay thin and declare the same implementation files;
component contracts remain independent and execution models connect them through adapters.

The component READMEs stay beside their manifests. Standalone entry points may
include those READMEs for documentation; embedded entry points use short Rust doc
comments instead. This directory contains only source links and this README.
Runtime also links `tests/controlled/` because its source includes those Rust
fixtures by relative path. Preserve that relationship when adding fixtures.

Keep the links and this README in Git. Git stores each link's target path, not a
second copy of the implementation. Ignoring them would make fresh checkouts fail
to build. Do not put component manifests here or link entire component packages:
that would reintroduce Cargo's nested-package exclusion.

## What consumers receive

The published archive contains ordinary source files at these paths, not links.
Consumers need neither symlink support nor the original `crates/` directories.
The proc-macro companion, `contime-macros`, remains separately compiled and
published; it is not assembled through this directory.

Cargo documents [symlink flattening](https://doc.rust-lang.org/cargo/commands/cargo-package.html)
and [nested-package exclusions](https://doc.rust-lang.org/cargo/reference/manifest.html#the-exclude-and-include-fields).
This directory layout is ConTime's use of those supported behaviors.

## Checkout and verification

Development checkouts must preserve symbolic links. Some Windows setups require
Developer Mode and Git symlink support (`core.symlinks=true`) before cloning.
See [Git's symlink configuration](https://git-scm.com/docs/git-config#Documentation/git-config.txt-coresymlinks).
If a link appears as a small text file, fix the checkout configuration rather
than copying source files or editing the link text.

CI checks the link targets and archive contents, extracts both the public package
and its macro companion outside the checkout, then tests the extracted source.
It also maps coverage paths back to the authoritative files under `crates/`.
Standalone component tests and benchmarks continue to run through commands such
as `cargo test -p contime-worker` and `cargo bench -p contime-batcher --bench batcher`.
