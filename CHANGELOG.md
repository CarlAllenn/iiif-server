# Changelog

All notable changes to this project are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
over the surface named in [MAINTENANCE.md](MAINTENANCE.md) — the HTTP API, the
CLI flags and the container contract. Internal Rust APIs are not covered:
nothing in this workspace is published to a registry.

## [0.1.0](https://github.com/CarlAllenn/iiif-server/releases/tag/v0.1.0) - 2026-08-01

### Added

- workspace skeleton — typed grammar, identifier rules, source seam
- the first pixel path — pyramid TIFF to HTTP tile
- official IIIF validator wired from git — v3 level 2 green, 33/33
- JPEG-in-TIFF verified — and it caught a real color bug
- j2k vs OpenJPEG — bit-exact, fast, Plan B stays shelved
- object-store range-read profile vs MinIO
- M1 Link headers + HTTP semantics tests; mimalloc ships (bench-decided)
- the codec seam — JP2/HTJ2K serving, plain JPEG/PNG masters
- the v2.1 endpoint — both official validators green
- the completionist sweep — full compliance table shipped
- HTTP caching correctness — ETags, conditionals, Cache-Control
- /metrics — the frozen observability surface
- object-store serving — s3:// roots, pixel-verified e2e
- iiif-server check — offline master inspection
- fuzzing — and it found a real 25 GB decompression bomb
- adopt shared preset (github>CarlAllenn/renovate-config) (#17) ([#17](https://github.com/CarlAllenn/iiif-server/pull/17))
- every tile grid takes the region fast path (#22) ([#22](https://github.com/CarlAllenn/iiif-server/pull/22))
- task baseline de-drift — ci umbrella, per-tool lint tasks, fingerprint every gate (#24) ([#24](https://github.com/CarlAllenn/iiif-server/pull/24))
- lefthook baseline — universal lint layer consumed live via remotes (#27) ([#27](https://github.com/CarlAllenn/iiif-server/pull/27))
- adopt cargo-deny supply-chain baseline (renovate-config#5) (#30) ([#30](https://github.com/CarlAllenn/iiif-server/pull/30))
- harden-runner block mode (renovate-config#6) (#31) ([#31](https://github.com/CarlAllenn/iiif-server/pull/31))
- scanner baseline — gitleaks, machete, lychee (#7) (#32) ([#32](https://github.com/CarlAllenn/iiif-server/pull/32))
- #8 tooling baseline — full lint canon, rustfmt, fuzz CI, SBOM (#34) ([#34](https://github.com/CarlAllenn/iiif-server/pull/34))
- JP2 zoom-outs decode to the ladder's full depth, not the 1/8 cap (#40) ([#40](https://github.com/CarlAllenn/iiif-server/pull/40))
- official container image, release pipeline, and installer (#49) ([#49](https://github.com/CarlAllenn/iiif-server/pull/49))

### CI

- lint+test on x86-64 and arm64, digest-pinned, MSRV job
- validator job name reflects that both API versions run

### Dependencies

- pin dependencies (#6) ([#6](https://github.com/CarlAllenn/iiif-server/pull/6))
- update actions/upload-artifact digest to 043fb46 (#7) ([#7](https://github.com/CarlAllenn/iiif-server/pull/7))
- update dependency aqua:taiki-e/cargo-llvm-cov to v0.8.7 (#35) ([#35](https://github.com/CarlAllenn/iiif-server/pull/35))

### Documentation

- founding design spec from scoping session
- amend founding spec — full dependency evaluation, public-from-start
- license decision — AGPL-3.0-only, CLA for external contributions
- close M0 ambiguities — CLA mechanism, test-fixture provenance
- AGPL-3.0-only license + Apache-ICLA-derived CLA
- reflow prose to 120 columns for markdownlint
- session build report
- record the filed upstream issue and the fast-path-only caveat (#5) ([#5](https://github.com/CarlAllenn/iiif-server/pull/5))
- head-to-head eval vs Cantaloupe — the partial-grid numbers (#11) ([#11](https://github.com/CarlAllenn/iiif-server/pull/11))
- freshness pass — three catches not one, gate revisit happened, j2k bug filed (#16) ([#16](https://github.com/CarlAllenn/iiif-server/pull/16))
- drop stale fallback-path wording in eval corpus comment (#23) ([#23](https://github.com/CarlAllenn/iiif-server/pull/23))
- ci badge in README (#26) ([#26](https://github.com/CarlAllenn/iiif-server/pull/26))
- eval rerun after the decode-stack changes; sharpen the HTJ2K claim (#42) ([#42](https://github.com/CarlAllenn/iiif-server/pull/42))

### Fixed

- conda fixture tools get their own config root
- validator script installs fixture tools explicitly
- never fingerprint lint:deny — RustSec DB is an unseen input; weekly scheduled gate (#25) ([#25](https://github.com/CarlAllenn/iiif-server/pull/25))
- drop en-GB_to_en-US dictionary — the house dialect is en-GB (#29) ([#29](https://github.com/CarlAllenn/iiif-server/pull/29))
- taplo-stable comment style in [tools] — Renovate bumps broke alignment (#36) ([#36](https://github.com/CarlAllenn/iiif-server/pull/36))
- resource-ceiling refusals answer 403, not 500 corrupt master (#12) (#39) ([#39](https://github.com/CarlAllenn/iiif-server/pull/39))
- sign the Release PR commit via the GitHub API (#51) ([#51](https://github.com/CarlAllenn/iiif-server/pull/51))
- lowercase the image name; reset for a clean 0.1.0 (#54) ([#54](https://github.com/CarlAllenn/iiif-server/pull/54))

### Miscellaneous

- tooling standup — pinned, locked, max enforcement
- port the proven pieces from edtf and monumental-archive (#9) ([#9](https://github.com/CarlAllenn/iiif-server/pull/9))
- rust toolchain surfaces as a PR, not a dashboard line (#10) ([#10](https://github.com/CarlAllenn/iiif-server/pull/10))
- adopt max-enforcement linter baseline from renovate-config (#28) ([#28](https://github.com/CarlAllenn/iiif-server/pull/28))
- de-drift the mise settings block to canon (#43) ([#43](https://github.com/CarlAllenn/iiif-server/pull/43))

### Performance

- adaptive codec parallelism; M2 libvips gate measured honestly
