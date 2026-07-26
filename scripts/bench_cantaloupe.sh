#!/bin/sh
# Head-to-head latency: iiif-server vs Cantaloupe (docs/bench/cantaloupe-eval.md).
#
# Both servers are measured as full HTTP round trips in release/production
# configuration — server vs server, no subtractions. Three passes:
#   warm      — steady state, BENCH_REPS reps per case (default 30)
#   cold      — fresh process/container + empty caches, first request,
#               COLD_REPS restarts (default 5)
#   cache-on  — Cantaloupe with its filesystem derivative cache enabled,
#               steady state (cache hits): the incumbent's deployed posture
#
# Prerequisites: scripts/gen_eval_corpus.sh has run, and the Cantaloupe
# eval image (docker build -t cantaloupe-eval:openjpeg tools/bench/cantaloupe)
# is available as ${CANTALOUPE_IMAGE:-cantaloupe-eval:openjpeg}, with the
# eval properties directory in ${CANTALOUPE_CONF:?}.
set -eu

cd "$(dirname "$0")/.."
root_dir=$(pwd)
gen=${root_dir}/tests/fixtures/generated/eval
OUR_PORT=6970
CANT_PORT=18183
REPS=${BENCH_REPS:-30}
COLD_REPS=${COLD_REPS:-5}
IMAGE=${CANTALOUPE_IMAGE:-cantaloupe-eval:openjpeg}
# Brace-free message: a } inside ${:?...} would close the expansion early.
CONF=${CANTALOUPE_CONF:?set CANTALOUPE_CONF to the dir holding the eval properties files}
# all | warm-ours | warm-cant-nocache | warm-cant-cache | cold
PASS=${BENCH_PASS:-all}

if [ ! -f "${gen}/scan_partial_ll.jp2" ]; then
    echo "run scripts/gen_eval_corpus.sh first" >&2
    exit 2
fi

cargo build --release -p iiif-server >/dev/null

ours_start() {
    ./target/release/iiif-server serve "${gen}" --bind "127.0.0.1:${OUR_PORT}" \
        --max-width 20000 --max-height 20000 --max-area 400000000 \
        >/dev/null 2>&1 &
    ours_pid=$!
    for _ in $(seq 1 100); do
        curl -sf "http://127.0.0.1:${OUR_PORT}/healthz" >/dev/null 2>&1 && return 0
        sleep 0.1
    done
    echo "iiif-server failed to start" >&2
    return 1
}
ours_stop() {
    [ "${ours_pid}" = 0 ] && return 0
    kill "${ours_pid}" 2>/dev/null || true
    wait "${ours_pid}" 2>/dev/null || true
    ours_pid=0
}

cant_start() { # cant_start <properties-file>
    docker run -d --name bench-cant -m 4g -p "127.0.0.1:${CANT_PORT}:8182" \
        -v "${gen}:/imageroot:ro" \
        -v "$1:/opt/cantaloupe/cantaloupe.properties:ro" \
        "${IMAGE}" \
        java -Xmx2g -Dcantaloupe.config=/opt/cantaloupe/cantaloupe.properties \
        -jar /opt/cantaloupe/cantaloupe.jar >/dev/null
    for _ in $(seq 1 180); do
        curl -sf "http://127.0.0.1:${CANT_PORT}/health" >/dev/null 2>&1 && return 0
        sleep 0.5
    done
    echo "cantaloupe failed to start" >&2
    return 1
}
cant_stop() { docker rm -f bench-cant >/dev/null 2>&1 || true; }

cleanup() {
    ours_stop
    cant_stop
}
trap cleanup EXIT
ours_pid=0
cant_stop # remove any leftover container from an aborted run

platform=$(uname -sm) || platform=unknown
cpu=$(sysctl -n machdep.cpu.brand_string 2>/dev/null) || cpu=""
if [ -z "${cpu}" ] && [ -r /proc/cpuinfo ]; then
    cpu=$(sed -n 's/^model name[[:space:]]*: //p' /proc/cpuinfo | head -1) || cpu=""
fi
echo "hardware: ${platform}, ${cpu:-unknown CPU}"
echo "warm reps: ${REPS}; cold restarts: ${COLD_REPS}"
echo "cantaloupe: ${IMAGE}, -Xmx2g, 4g container, OpenJpegProcessor for jp2"
echo

bench_pass() { # bench_pass <label> <base-url> <mode: warm|cold>
    uv run --quiet --with numpy python - "$@" "${REPS}" <<'EOF'
import sys
import time
import urllib.error
import urllib.request

import numpy as np

label, base, mode, reps = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])

CASES = [
    ("TIFF pyr deflate, native 512² tile", "scan_pyr_deflate.tif", "2048,2048,512,512/max"),
    ("TIFF pyr deflate, full → 512 wide", "scan_pyr_deflate.tif", "full/512,"),
    ("TIFF pyr JPEG, native 512² tile", "scan_pyr_jpeg.tif", "2048,2048,512,512/max"),
    ("JP2 partial-grid lossless, native tile", "scan_partial_ll.jp2", "2048,2048,512,512/max"),
    ("JP2 partial-grid lossless, full → 512", "scan_partial_ll.jp2", "full/512,"),
    ("JP2 partial-grid 20:1 lossy, native tile", "scan_partial_r20.jp2", "2048,2048,512,512/max"),
    ("JP2 exact-grid lossless, native tile", "exact_ll.jp2", "2048,2048,512,512/max"),
    ("JP2 untiled+precincts, native tile", "scan_untiled_ll.jp2", "2048,2048,512,512/max"),
    ("JP2 large 165MP partial, native tile", "large_partial_ll.jp2", "2048,2048,512,512/max"),
    ("JP2 large 165MP partial, full → 512", "large_partial_ll.jp2", "full/512,"),
    ("HTJ2K partial-grid lossless, native tile", "scan_ht_ll.j2c", "2048,2048,512,512/max"),
    ("HTJ2K partial-grid lossless, full → 512", "scan_ht_ll.j2c", "full/512,"),
    ("HTJ2K lossy, native tile", "scan_ht_lossy.j2c", "2048,2048,512,512/max"),
    ("HTJ2K large 165MP, native tile", "large_ht_ll.j2c", "2048,2048,512,512/max"),
    ("plain JPEG 28MP, full → 512", "scan_plain.jpg", "full/512,"),
    ("plain JPEG 28MP, native 512² tile", "scan_plain.jpg", "2048,2048,512,512/max"),
]


def fetch(url):
    start = time.perf_counter()
    try:
        with urllib.request.urlopen(url, timeout=300) as resp:
            resp.read()
            code = resp.status
    except urllib.error.HTTPError as e:
        e.read()
        code = e.code
    return time.perf_counter() - start, code


for case_label, ident, params in CASES:
    url = f"{base}/iiif/3/{ident}/{params}/0/default.jpg"
    if mode == "warm":
        _, code = fetch(url)  # warm-up, also detects unsupported
        if code != 200:
            print(f"{label}\t{case_label}\tHTTP {code}\t")
            continue
        samples = [fetch(url)[0] for _ in range(reps)]
        arr = np.array(samples) * 1000.0
        p50, p99 = np.percentile(arr, 50), np.percentile(arr, 99)
        print(f"{label}\t{case_label}\t{p50:.2f}\t{p99:.2f}")
    else:  # cold: single first-request measurement (caller restarts server)
        dur, code = fetch(url)
        if code != 200:
            print(f"{label}\t{case_label}\tHTTP {code}\t")
        else:
            print(f"{label}\t{case_label}\t{dur * 1000.0:.2f}\t")
EOF
}

if [ "${PASS}" = all ] || [ "${PASS}" = warm-ours ]; then
    echo "=== warm pass: iiif-server ==="
    ours_start
    bench_pass "ours-warm" "http://127.0.0.1:${OUR_PORT}" warm
    ours_stop
fi

if [ "${PASS}" = all ] || [ "${PASS}" = warm-cant-nocache ]; then
    echo "=== warm pass: cantaloupe, derivative cache OFF ==="
    cant_start "${CONF}/eval-cache-off.properties"
    bench_pass "cant-nocache-warm" "http://127.0.0.1:${CANT_PORT}" warm
    cant_stop
fi

if [ "${PASS}" = all ] || [ "${PASS}" = warm-cant-cache ]; then
    echo "=== warm pass: cantaloupe, derivative cache ON (cache hits) ==="
    cant_start "${CONF}/eval-cache-on.properties"
    bench_pass "cant-cache-warm" "http://127.0.0.1:${CANT_PORT}" warm
    cant_stop
fi

if [ "${PASS}" = all ] || [ "${PASS}" = cold ]; then
    echo "=== cold pass (${COLD_REPS} fresh starts each) ==="
    i=1
    while [ "${i}" -le "${COLD_REPS}" ]; do
        ours_start
        bench_pass "ours-cold-${i}" "http://127.0.0.1:${OUR_PORT}" cold
        ours_stop
        i=$((i + 1))
    done
    i=1
    while [ "${i}" -le "${COLD_REPS}" ]; do
        cant_start "${CONF}/eval-cache-off.properties"
        bench_pass "cant-cold-${i}" "http://127.0.0.1:${CANT_PORT}" cold
        cant_stop
        i=$((i + 1))
    done
fi
