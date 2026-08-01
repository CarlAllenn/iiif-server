# syntax=docker/dockerfile:1@sha256:87999aa3d42bdc6bea60565083ee17e86d1f3339802f543c0d03998580f9cb89

# The official image: one static musl binary and a certificate bundle, on
# nothing at all. No distro, no shell, no package manager, no writable
# filesystem — the container-shaped twin of "one static binary".
#
# Base images are digest-pinned and Renovate-tracked. The builder is
# rust:alpine because its toolchain is musl-native: the binary it produces
# needs no shared libraries, which is what makes the scratch final stage
# possible.

FROM rust:1.97.1-alpine3.22@sha256:df4efa4e0cdfb5245fa06e3f431387b2bcc96782ce5681b7fb6b0297d745bc29 AS build

# gcc and musl-dev already ship in rust:alpine — mimalloc's C needs them.
# cargo-auditable is the only addition, and it is load-bearing: Rust discards
# dependency information at compile time, so a scanner pointed at a stripped
# static binary sees one file and zero packages. cargo-auditable embeds a
# compressed dependency list that syft, trivy and grype all read. Without it
# every SBOM this image ever carries would be empty.
# hadolint ignore rationale for the unpinned apk version lives in
# .hadolint.yaml (DL3018) — this stage ships nothing.
RUN apk add --no-cache cargo-auditable

WORKDIR /src
COPY . .

# Injected by the release pipeline; reported by `--version` and the
# iiif_build_info metric. Empty in a local build, where the honest answer is
# "unknown" — a working tree is not a revision.
ARG IIIF_BUILD_REVISION=""
ENV IIIF_BUILD_REVISION=${IIIF_BUILD_REVISION}

# --locked: the committed lockfile is the reviewed dependency set, and a
# release must not resolve anything new. The binary is moved out inside the
# same layer because the target directory is a cache mount and does not
# survive into the next one. Deliberately not stripped: `strip` discards the
# non-allocated section cargo-auditable writes, which would silently undo
# the SBOM above.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo auditable build --release --locked --bin iiif-server \
    && mv target/release/iiif-server /iiif-server

FROM scratch

# The TLS trust store. rustls reads it through SSL_CERT_FILE, which is what
# lets a scratch image talk to S3 at all: there is no operating system here
# to hold a default bundle, so the bundle is shipped as a file. Serving local
# files would work without it; every s3:// deployment would not.
COPY --from=build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=build /src/LICENSE /LICENSE
COPY --from=build /iiif-server /iiif-server

ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt

# Numeric, because scratch has no /etc/passwd to name a user in. 65532 is the
# conventional non-root uid for distroless-style images.
USER 65532:65532

EXPOSE 6363

# The image holds no shell and no curl, so the binary probes itself.
HEALTHCHECK --interval=10s --timeout=5s --retries=6 --start-period=5s \
    CMD ["/iiif-server", "healthcheck", "127.0.0.1:6363"]

ENTRYPOINT ["/iiif-server"]
CMD ["serve", "/imageroot", "--bind", "0.0.0.0:6363"]
