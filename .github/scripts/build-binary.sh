#!/bin/sh
# Build a native macOS binary for the release. The Linux binaries come out of
# the container image instead (see extract-binary.sh).
#
# cargo-auditable rather than plain cargo build, for the same reason the image
# uses it: Rust discards dependency information at compile time, so `cargo
# audit bin` and every scanner would otherwise see a downloaded binary as one
# opaque file with no dependencies.
set -eu

cd "$(dirname "$0")/../.."

: "${TARGET:?TARGET must be set}"
: "${GITHUB_REF:?GITHUB_REF must be set (this runs on a tag)}"

rustup target add "${TARGET}"
cargo install cargo-auditable --locked

version=${GITHUB_REF#refs/tags/}
name="iiif-server-${version}-${TARGET}"

cargo auditable build --release --locked --bin iiif-server --target "${TARGET}"

mkdir -p dist
cp "target/${TARGET}/release/iiif-server" dist/iiif-server
cp LICENSE dist/LICENSE
tar -czf "dist/${name}.tar.gz" -C dist iiif-server LICENSE
rm -f dist/iiif-server dist/LICENSE

(cd dist && shasum -a 256 "${name}.tar.gz" > "${name}.tar.gz.sha256")
echo "packaged ${name}.tar.gz"
