#!/bin/sh
# Lift the static binary out of the image that was just built and smoke
# tested, and package it for the release.
#
# Not a second build: this is byte-identical to what runs in the container,
# which is the only way `--version` output from a bug report can be matched
# against a downloaded binary with certainty.
set -eu

cd "$(dirname "$0")/../.."

: "${TARGET:?TARGET must be set}"
: "${GITHUB_REF:?GITHUB_REF must be set (this runs on a tag)}"

version=${GITHUB_REF#refs/tags/}
name="iiif-server-${version}-${TARGET}"

mkdir -p dist
container=$(docker create iiif-server:candidate)
docker cp "${container}:/iiif-server" "dist/iiif-server"
docker rm "${container}" > /dev/null

# A tarball rather than a bare binary: it preserves the executable bit, which
# a browser download would otherwise strip, and it carries the licence with
# the artifact as AGPL redistribution expects.
cp LICENSE dist/LICENSE
tar -czf "dist/${name}.tar.gz" -C dist iiif-server LICENSE
rm -f dist/iiif-server dist/LICENSE

(cd dist && sha256sum "${name}.tar.gz" > "${name}.tar.gz.sha256")
echo "packaged ${name}.tar.gz"
