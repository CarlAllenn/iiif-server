#!/bin/sh
# Run the official IIIF validators against a container image.
#
# The point of the indirection: scripts/validate.sh normally builds a host
# binary and validates that. What ships is a musl static binary, built through
# cargo-auditable, inside a scratch image, with its own entrypoint and user —
# same source, different artifact. Publishing "33/33" should mean the bytes
# people pull, so the release pipeline runs the validators this way.
#
# usage: scripts/validate_image.sh [IMAGE_REF] [PORT]
set -eu

image=${1:-iiif-server:local}
port=${2:-6404}
container=iiif-validate-$$

cd "$(dirname "$0")/.."
root_dir=$(pwd)

cleanup() {
  docker rm -f "${container}" > /dev/null 2>&1 || true
}
trap cleanup EXIT

# validate.sh generates the reference master itself (libvips, via mise) before
# it validates anything, and writes it into this directory — which is bind
# mounted, so it appears inside the running container. The directory must
# exist first, or Docker creates it root-owned.
gen=${root_dir}/tests/fixtures/generated
mkdir -p "${gen}"

docker run -d --name "${container}" -p "${port}:6363" \
  -v "${gen}:/imageroot:ro" \
  "${image}" > /dev/null

for _ in $(seq 1 50); do
  if curl -sf "http://127.0.0.1:${port}/healthz" > /dev/null 2>&1; then
    break
  fi
  sleep 0.2
done

# Deliberately not exec: the cleanup trap has to survive to remove the
# container.
scripts/validate.sh --server "127.0.0.1:${port}"
