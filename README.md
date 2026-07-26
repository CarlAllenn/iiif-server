# iiif-server (working name)

A modern IIIF Image API server: **3.0 and 2.1, level 2 plus the complete optional feature table**, pure Rust — including JPEG 2000/HTJ2K decode — one static binary, zero C parsing untrusted input, stateless, scope-frozen at 1.0.

**Status: design phase.** No code yet. The founding document is [docs/design-spec.md](docs/design-spec.md) — the decided spec from the 2026-07-26 scoping session, amended the same day by a full dependency evaluation (crate sources inspected; JP2 resolved to the pure-Rust `j2k`). It is self-contained: tooling standup and the build proceed from it, starting at milestone M0 (skeleton + the de-risking spikes).

The repo name is a placeholder; product naming, org, and domain are deliberately deferred to the launch milestone. Nothing gets published to crates.io, Docker Hub, or any registry until then. The repo is public from the start (public-repo CI is free, including arm64 runners) — public is not launched; the first announcement comes with the name.
