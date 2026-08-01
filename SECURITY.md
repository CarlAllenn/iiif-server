# Security policy

## Reporting a vulnerability

Report privately through GitHub's [security advisory
form](https://github.com/CarlAllenn/iiif-server/security/advisories/new). That
keeps the report private until a fix ships and gives you credit on the
published advisory.

Please do not open a public issue for a suspected vulnerability.

If you cannot use GitHub advisories, email the maintainer address on the
commits in this repository.

## What to expect

Advisories against the tracked dependency class, and defects in this code, are
triaged within **7 days** and released as signed patch releases. That window
is the commitment made in [MAINTENANCE.md](MAINTENANCE.md), which also
explains why a project that is feature-complete by design still needs a
security response process.

## Supported versions

The latest release only. This project is scope-frozen by design rather than
long-lived-branch-maintained: fixes land on the current version and a new
patch release follows.

## The security property, and its edges

**Zero C parses untrusted input, anywhere in the product.** Request URLs are
hostile and are fuzzed — the grammar and identifier resolution both. Source
masters are operator-curated and semi-trusted: a malicious master is an
availability problem, not remote code execution, because every decoder is
memory-safe Rust.

Things that are therefore *not* vulnerabilities in this project, and will be
closed as such:

- **Resource exhaustion from a master an operator chose to serve.** The
  bounded decode pool, the published `maxWidth`/`maxHeight`/`maxArea` limits
  and the decompression-bomb guards are the mitigation; a master that decodes
  slowly is a capacity question.
- **Anything requiring access to the source root.** An operator who can write
  to the image directory is already inside the trust boundary.
- **Missing authentication or TLS.** Both are deliberately out of scope and
  belong to the reverse proxy; see MAINTENANCE.md's pre-refusals and the
  forward-auth recipe in [docs/deployment.md](docs/deployment.md).

Reports in those categories are still welcome as issues if the behaviour looks
wrong — they are just not embargoed.

## Verifying what you run

Released images and binaries are signed with cosign keylessly and carry SLSA
build provenance. The signing identity is the publishing workflow at the tag,
so you can require exactly that:

```console
gh attestation verify oci://ghcr.io/carlallenn/iiif-server@sha256:… \
  --repo CarlAllenn/iiif-server
```

```console
cosign verify ghcr.io/carlallenn/iiif-server@sha256:… \
  --certificate-identity-regexp '^https://github.com/CarlAllenn/iiif-server/\.github/workflows/publish\.yml@refs/tags/v' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

A signature nobody checks does no work — if you deploy this, wire one of those
into your pipeline.
