#!/bin/sh
# SPIKE 1 fixtures: JPEG-in-TIFF (ModernJPEG, tag 7) pyramids, including
# subsampled YCbCr and shared JPEGTables — the M0 de-risking spike for the
# tiff crate's JPEG delegation to zune-jpeg.
#
# Produces, under tests/fixtures/generated/:
#   spike1_ycbcr420.tif  — Q75: libvips subsamples chroma (4:2:0) below Q90
#   spike1_ycbcr444.tif  — Q95: no chroma subsampling at/above Q90
#   spike1_golden_*.ppm  — libvips (libjpeg) decodes of test regions, the
#                          golden reference for cross-decoder comparison
set -eu

cd "$(dirname "$0")/.."
root_dir=$(pwd)
gen=${root_dir}/tests/fixtures/generated
mkdir -p "${gen}"
tmp=$(mktemp -d)
trap 'rm -rf "${tmp}"' EXIT

# Smooth photographic-ish pattern: JPEG-friendly (bounded error), position
# identifiable (gradients + low-frequency color waves).
python3 - "${tmp}/smooth.ppm" <<'EOF'
import math
import sys

W, H = 2048, 1536
rows = bytearray()
for y in range(H):
    for x in range(W):
        r = int(127.5 + 127.5 * math.sin(x / 97.0))
        g = int(127.5 + 127.5 * math.sin(y / 71.0))
        b = int(127.5 + 127.5 * math.sin((x + y) / 53.0))
        rows += bytes((r, g, b))
with open(sys.argv[1], "wb") as f:
    f.write(b"P6\n%d %d\n255\n" % (W, H))
    f.write(rows)
EOF

vips() {
    mise --cd "${root_dir}/tools/fixtures" exec conda:libvips -- vips "$@"
}

vips tiffsave "${tmp}/smooth.ppm" "${gen}/spike1_ycbcr420.tif" \
    --tile --tile-width 256 --tile-height 256 --pyramid \
    --compression jpeg --Q 75

vips tiffsave "${tmp}/smooth.ppm" "${gen}/spike1_ycbcr444.tif" \
    --tile --tile-width 256 --tile-height 256 --pyramid \
    --compression jpeg --Q 95

# Golden decodes of the SAME JPEG-TIFFs via libvips/libjpeg: level-0
# regions crossing tile boundaries, written as raw PPM.
for name in ycbcr420 ycbcr444; do
    vips crop "${gen}/spike1_${name}.tif" "${tmp}/crop.v" 192 192 384 384
    vips ppmsave "${tmp}/crop.v" "${gen}/spike1_golden_${name}_192_192_384_384.ppm"
    vips crop "${gen}/spike1_${name}.tif" "${tmp}/crop2.v" 0 0 256 256
    vips ppmsave "${tmp}/crop2.v" "${gen}/spike1_golden_${name}_0_0_256_256.ppm"
done

echo "spike1 fixtures:"
ls -la "${gen}"/spike1_*
