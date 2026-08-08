# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A complete, scope-frozen implementation of the IIIF Image API (3.0 + 2.1, level 2 plus all optional
features) as one static Rust binary. Pure Rust everywhere untrusted input is parsed — zero C in the
product (dev-time fixture generation is the only exemption). AGPL-3.0-only; external PRs go through
a CLA.

The feature surface is deliberately finished: [docs/design-spec.md](docs/design-spec.md) lists
pre-refusals (AVIF/JXL, auth, Presentation API, per-image metadata) that are declined in advance.
Do not add features; correctness, security, docs, and test coverage are the welcome categories.

## Commands

Tooling is pinned with mise: `mise install` (also installs git hooks via lefthook). All workflows go
through go-task:

- `task ci` — everything CI runs (lint + test), identical locally and on GitHub
- `task test` — full test suite (`cargo test --workspace --all-features`)
- `cargo test -p iiif-core --test <name> <filter>` — a single test/integration file
- `task lint` — every linter; `task lint:rust` for just rustfmt-check + clippy
- `task fmt` — apply all formatters (rustfmt on pinned nightly, taplo, shfmt)
- `task fixtures:gen` — regenerate committed test masters (needs libvips via mise)
- `task validate` — official IIIF validators against a local build
- `task fuzz` — brief run of all fuzz targets (`FUZZ_SECONDS` to extend)
- `task image` / `task image:smoke` — build and smoke-test the container

Lint/test tasks are checksum-fingerprinted; unchanged inputs skip. `task --force` overrides.
Security gates (`lint:deny`, `scan:image`) are deliberately never fingerprinted.

## Ground rules

- **No lint exemptions, ever.** The lint set is maximal (clippy all/pedantic/nursery/cargo +
  restriction opt-ins, `-D warnings`, `unsafe_code = "forbid"`). Fix the code; never add `#[allow]`
  or config carve-outs. The few recorded allows in Cargo.toml are canonical decisions from the
  renovate-config template — don't extend them.
- **Main is protected; all changes land by PR.** No direct pushes.
- **Lint canon lives in CarlAllenn/renovate-config templates** (rust-lints, rustfmt, mise settings,
  trivy, etc.). Those blocks are drift-audited — don't edit them locally; change the template and
  propagate.
- **No closing keywords next to cross-repo references** in PR/commit prose ("fixes owner/repo#N"
  auto-closes the other repo's issue on merge). Use descriptive wording.
- **Dependencies must survive cargo-deny**: permissive licenses only, wildcard bans, reasoned skips
  for duplicate versions.
- The fuzz workspace (`fuzz/`) is excluded from the main workspace, builds on nightly with its own
  committed `Cargo.lock`, and is gated `--locked` in `lint:rust`.
- The pinned nightly rustfmt toolchain lives in one place: `RUSTFMT_TOOLCHAIN` in Taskfile.yml.
  Bump it manually alongside stable toolchain bumps.

## Architecture

Three workspace crates:

- **`crates/core` (iiif-core)** — everything spec-shaped and pixel-shaped: URL grammar
  parsing/printing ([grammar.rs](crates/core/src/grammar.rs), property-tested for round-trips and
  canonicalization), request evaluation ([eval.rs](crates/core/src/eval.rs)), the
  decode→transform→encode pipeline ([pipeline.rs](crates/core/src/pipeline.rs)), codec integrations
  under `codec/`, info.json generation ([info.rs](crates/core/src/info.rs)), and the 2.1
  compatibility layer ([v2.rs](crates/core/src/v2.rs)). The `Source` abstraction
  ([source.rs](crates/core/src/source.rs)) is the seam to storage.
- **`crates/sources` (iiif-sources)** — storage backends: local filesystem and S3-compatible object
  stores (range reads; GCS/Azure work by S3 compatibility).
- **`crates/server` (iiif-server)** — the axum HTTP layer: routing/headers/conneg
  ([app.rs](crates/server/src/app.rs)), bounded decode pool with 503 + Retry-After backpressure,
  `/healthz` and Prometheus `/metrics`, and the CLI (`serve`, `check`).

Correctness is enforced three ways and new work should slot into them: official IIIF validators in
CI, golden/differential tests pinning pixels against libvips/libjpeg/OpenJPEG (masters are
generated, gitignored, digest-verified — `task fixtures:gen`), and property tests over the grammar.
Correctness fixes want a failing test first.

Performance-sensitive decode paths (notably JPEG 2000 via the `j2k` crate) have benchmark harnesses
under `tools/bench/` and `task bench`; numbers get recorded in `docs/`, raw spike output is
gitignored.
