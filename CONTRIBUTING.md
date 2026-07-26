# Contributing

Thanks for your interest. Two things to know before you open a PR.

## License and CLA

The project is licensed [AGPL-3.0-only](LICENSE). External contributions
require agreeing to the [Individual Contributor License Agreement](CLA.md)
(Apache ICLA-derived, including a relicensing grant to the maintainer). The
CLA-assistant bot will prompt you on your first pull request; agreement is
recorded once and covers subsequent contributions.

## Scope

Read [docs/design-spec.md](docs/design-spec.md) first. The feature surface is
deliberately complete-and-frozen: the full IIIF Image API 3.0 and 2.1 level 2
compliance tables, nothing else. The spec's "Pre-refusals" section lists
features that are declined in advance (AVIF/JXL outputs, auth in the engine,
Presentation API, per-image metadata, …) — PRs adding them will be closed
with a pointer to that section, kindly.

What is always welcome:

- correctness fixes, with a failing test first
- security fixes
- documentation accuracy
- new golden/property/fuzz coverage

## Development

Tooling is pinned with [mise](https://mise.jdx.dev): `mise install`, then
`task ci` runs exactly what CI runs (fmt + clippy + cargo-deny + linters +
tests). `lefthook install` wires the same checks as git hooks.

The workspace is `#![forbid(unsafe_code)]` throughout, and every dependency
must be permissively licensed (enforced by `cargo deny`). Zero C code parses
untrusted input anywhere in the product — dev-time fixture generation is the
only exemption.
