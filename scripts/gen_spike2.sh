#!/bin/sh
# SPIKE 2 fixtures: pyramidal JP2 masters + OpenJPEG golden decodes.
#
# Produces, under tests/fixtures/generated/:
#   spike2_lossless.jp2   — 8192×8192, reversible 5/3, 1024px tiles, 6 levels
#   spike2_lossy.jp2      — same geometry, irreversible 9/7 at 20:1
#   spike2_lossless4k.jp2 — 4096×4096 variant (fits j2k's recode allocation
#                           cap; used for the HTJ2K recode leg)
#   spike2_golden_*.ppm — opj_decompress outputs (the golden reference):
#     region 3072,2560 → 3584,3072 at full resolution, from both masters
#     full image at reduction 2 (quarter resolution), from both masters
set -eu

cd "$(dirname "$0")/.."
root_dir=$(pwd)
gen=${root_dir}/tests/fixtures/generated
mkdir -p "${gen}"
tmp=$(mktemp -d)
trap 'rm -rf "${tmp}"' EXIT

opj() {
    tool=$1
    shift
    mise --cd "${root_dir}/tools/fixtures" exec conda:openjpeg -- "${tool}" "$@"
}

if [ ! -f "${gen}/spike2_lossless.jp2" ] || [ ! -f "${gen}/spike2_lossy.jp2" ]; then
    # 8192×8192 smooth deterministic pattern; numpy keeps generation fast.
    uv run --quiet --with numpy python - "${tmp}/big.ppm" <<'EOF'
import sys

import numpy as np

N = 8192
x = np.arange(N, dtype=np.float64)
y = x[:, None]
r = 127.5 + 127.5 * np.sin(x / 211.0)
g = 127.5 + 127.5 * np.sin(y / 173.0)
b = 127.5 + 127.5 * np.sin((x + y) / 101.0)
img = np.empty((N, N, 3), dtype=np.uint8)
img[..., 0] = np.broadcast_to(r, (N, N)).round()
img[..., 1] = np.broadcast_to(g, (N, N)).round()
img[..., 2] = b.round()
with open(sys.argv[1], "wb") as f:
    f.write(b"P6\n%d %d\n255\n" % (N, N))
    f.write(img.tobytes())
EOF

    opj opj_compress -i "${tmp}/big.ppm" -o "${gen}/spike2_lossless.jp2" \
        -t 1024,1024 -n 6 >/dev/null
    opj opj_compress -i "${tmp}/big.ppm" -o "${gen}/spike2_lossy.jp2" \
        -t 1024,1024 -n 6 -r 20 -I >/dev/null
fi

if [ ! -f "${gen}/spike2_lossless4k.jp2" ]; then
    uv run --quiet --with numpy python - "${tmp}/mid.ppm" <<'EOF'
import sys

import numpy as np

N = 4096
x = np.arange(N, dtype=np.float64)
y = x[:, None]
r = 127.5 + 127.5 * np.sin(x / 211.0)
g = 127.5 + 127.5 * np.sin(y / 173.0)
b = 127.5 + 127.5 * np.sin((x + y) / 101.0)
img = np.empty((N, N, 3), dtype=np.uint8)
img[..., 0] = np.broadcast_to(r, (N, N)).round()
img[..., 1] = np.broadcast_to(g, (N, N)).round()
img[..., 2] = b.round()
with open(sys.argv[1], "wb") as f:
    f.write(b"P6\n%d %d\n255\n" % (N, N))
    f.write(img.tobytes())
EOF
    opj opj_compress -i "${tmp}/mid.ppm" -o "${gen}/spike2_lossless4k.jp2" \
        -t 1024,1024 -n 6 >/dev/null
    opj opj_decompress -i "${gen}/spike2_lossless4k.jp2" \
        -o "${tmp}/region_4k.ppm" -d 1024,1024,1536,1536 >/dev/null
    mv "${tmp}/region_4k.ppm" "${gen}/spike2_golden_lossless4k_region.ppm"
fi

# Golden decodes via opj_decompress. -d is in full-resolution reference
# grid coordinates; -r reduces by 2^n after region selection.
for variant in lossless lossy; do
    if [ ! -f "${gen}/spike2_golden_${variant}_region.ppm" ]; then
        opj opj_decompress -i "${gen}/spike2_${variant}.jp2" \
            -o "${tmp}/region_${variant}.ppm" \
            -d 3072,2560,3584,3072 >/dev/null
        mv "${tmp}/region_${variant}.ppm" "${gen}/spike2_golden_${variant}_region.ppm"
    fi
    if [ ! -f "${gen}/spike2_golden_${variant}_r2.ppm" ]; then
        opj opj_decompress -i "${gen}/spike2_${variant}.jp2" \
            -o "${tmp}/r2_${variant}.ppm" -r 2 >/dev/null
        mv "${tmp}/r2_${variant}.ppm" "${gen}/spike2_golden_${variant}_r2.ppm"
    fi
done

echo "spike2 fixtures:"
ls -la "${gen}"/spike2_*
