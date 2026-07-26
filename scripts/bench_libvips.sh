#!/bin/sh
# M2 benchmark gate: our tile-serving latency vs a libvips reference on
# the same masters and the same requests.
#
# Gate (design spec): p50 ≤ 1.5× libvips, p99 ≤ 2×. Hardware and corpus
# are printed with the numbers — a benchmark without them is a claim, not
# a measurement.
set -eu

cd "$(dirname "$0")/.."
root_dir=$(pwd)
gen=${root_dir}/tests/fixtures/generated
PORT=6969
REPS=${BENCH_REPS:-60}

if [ ! -f "${gen}/spike1_ycbcr420.tif" ]; then
    echo "run scripts/gen_spike1.sh first" >&2
    exit 2
fi
if [ ! -f "${gen}/spike2_lossless.jp2" ]; then
    echo "run scripts/gen_spike2.sh first" >&2
    exit 2
fi

cargo build --release -p iiif-server >/dev/null
./target/release/iiif-server serve "${gen}" --bind "127.0.0.1:${PORT}" \
    --max-width 20000 --max-height 20000 --max-area 400000000 &
server_pid=0
trap 'kill "${server_pid}" 2>/dev/null || true' EXIT
server_pid=$!
for _ in $(seq 1 50); do
    if curl -sf "http://127.0.0.1:${PORT}/healthz" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done

platform=$(uname -sm) || platform=unknown
cpu=$(sysctl -n machdep.cpu.brand_string 2>/dev/null) || cpu=""
if [ -z "${cpu}" ] && [ -r /proc/cpuinfo ]; then
    cpu=$(sed -n 's/^model name[[:space:]]*: //p' /proc/cpuinfo | head -1) || cpu=""
fi
echo "hardware: ${platform}, ${cpu:-unknown CPU}"
echo "reps per case: ${REPS}"
echo

uv run --quiet --with numpy python - "${PORT}" "${REPS}" "${gen}" <<'EOF'
import subprocess
import sys
import time
import urllib.request

import numpy as np

port, reps, gen = sys.argv[1], int(sys.argv[2]), sys.argv[3]


def stats(samples):
    array = np.array(samples) * 1000.0
    return float(np.percentile(array, 50)), float(np.percentile(array, 99))


def time_http(path):
    url = f"http://127.0.0.1:{port}{path}"
    urllib.request.urlopen(url).read()  # warm
    samples = []
    for _ in range(reps):
        start = time.perf_counter()
        urllib.request.urlopen(url).read()
        samples.append(time.perf_counter() - start)
    return stats(samples)


def time_vips(args):
    subprocess.run(args, check=True, capture_output=True)  # warm
    samples = []
    for _ in range(reps):
        start = time.perf_counter()
        subprocess.run(args, check=True, capture_output=True)
        samples.append(time.perf_counter() - start)
    return stats(samples)


def vips(*args):
    return [
        "mise", "--cd", f"{gen}/../../../tools/fixtures", "exec", "conda:libvips",
        "--", "vips", *args,
    ]


# The vips CLI pays process startup (mise shim + dynamic linking) on every
# invocation; our server pays it once. Measure that floor and subtract it,
# so the comparison is decode-vs-decode rather than decode-vs-fork.
STARTUP_P50, STARTUP_P99 = time_vips(vips("--version"))
print(f"libvips CLI startup floor (subtracted): p50 {STARTUP_P50:.2f} ms, "
        f"p99 {STARTUP_P99:.2f} ms")
print()


CASES = [
    (
        "pyramidal TIFF, 512² tile @ native",
        "/iiif/3/spike1_ycbcr420.tif/512,512,512,512/max/0/default.jpg",
        vips("crop", f"{gen}/spike1_ycbcr420.tif", "/tmp/bench_out.jpg[Q=85]",
                "512", "512", "512", "512"),
    ),
    (
        "pyramidal TIFF, full image → 512 wide",
        "/iiif/3/spike1_ycbcr420.tif/full/512,/0/default.jpg",
        vips("thumbnail", f"{gen}/spike1_ycbcr420.tif", "/tmp/bench_out.jpg[Q=85]", "512"),
    ),
    (
        "JP2 8192², 512² region @ native",
        "/iiif/3/spike2_lossless.jp2/3072,2560,512,512/max/0/default.jpg",
        vips("crop", f"{gen}/spike2_lossless.jp2", "/tmp/bench_out.jpg[Q=85]",
                "3072", "2560", "512", "512"),
    ),
]

print(f"{'case':<38} {'ours p50':>9} {'vips p50':>9} {'ratio':>7} "
        f"{'ours p99':>9} {'vips p99':>9} {'ratio':>7}  gate")
print("(vips columns are startup-subtracted: pure decode+encode work)")
failures = 0
for label, path, vips_args in CASES:
    ours_p50, ours_p99 = time_http(path)
    raw_p50, raw_p99 = time_vips(vips_args)
    vips_p50 = max(raw_p50 - STARTUP_P50, 0.01)
    vips_p99 = max(raw_p99 - STARTUP_P99, 0.01)
    r50, r99 = ours_p50 / vips_p50, ours_p99 / vips_p99
    ok = r50 <= 1.5 and r99 <= 2.0
    failures += 0 if ok else 1
    print(f"{label:<38} {ours_p50:8.2f}m {vips_p50:8.2f}m {r50:6.2f}x "
            f"{ours_p99:8.2f}m {vips_p99:8.2f}m {r99:6.2f}x  {'PASS' if ok else 'FAIL'}")

print()
print("gate: p50 <= 1.5x libvips, p99 <= 2x")
sys.exit(1 if failures else 0)
EOF
