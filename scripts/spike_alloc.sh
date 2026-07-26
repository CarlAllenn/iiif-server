#!/bin/sh
# M0 allocator bench: musl-native malloc vs mimalloc under concurrent
# decode, inside a musl Linux container (the shipping environment).
# Requires: Docker, and the spike1 fixture (scripts/gen_spike1.sh).
set -eu

cd "$(dirname "$0")/.."
work_dir=$(pwd)

FIXTURE=tests/fixtures/generated/spike1_ycbcr420.tif
if [ ! -f "${FIXTURE}" ]; then
    echo "run scripts/gen_spike1.sh first" >&2
    exit 2
fi

# rust:alpine = musl-native toolchain. Separate target dir keeps the
# container's musl artifacts away from the host's; a named volume caches
# the registry between runs.
docker run --rm \
    -v "${work_dir}":/work -w /work \
    -v iiif-alloc-bench-registry:/usr/local/cargo/registry \
    -e CARGO_TARGET_DIR=/tmp/target \
    rust:1.97-alpine sh -c '
        set -eu
        apk add --quiet build-base
        echo "--- building (system/musl allocator)"
        cargo build --release --example alloc_bench 2>/dev/null
        cp /tmp/target/release/examples/alloc_bench /tmp/bench_musl
        echo "--- building (mimalloc)"
        cargo build --release --features mimalloc --example alloc_bench 2>/dev/null
        cp /tmp/target/release/examples/alloc_bench /tmp/bench_mimalloc
        echo "--- 3 runs each (first is warmup)"
        for run in 1 2 3; do /tmp/bench_musl; done
        for run in 1 2 3; do /tmp/bench_mimalloc; done
    '
