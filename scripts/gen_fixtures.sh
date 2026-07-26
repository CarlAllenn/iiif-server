#!/bin/sh
# Regenerate the committed test masters in tests/fixtures/.
#
# Dev-time fixture generation is exempt from the zero-C doctrine (design
# spec, Quality regime): libvips runs here, never in the product. The
# pattern is deterministic — every pixel encodes its own coordinates
# (r = x mod 256, g = y mod 256, b marks the 256px block) — so tests can
# assert exact pixel values at any position.
#
# Requires: python3, and libvips (pinned in tools/fixtures/mise.toml).
set -eu

cd "$(dirname "$0")/.."
root_dir=$(pwd)
mkdir -p "${root_dir}/tests/fixtures"
tmp=$(mktemp -d)
trap 'rm -rf "${tmp}"' EXIT

python3 - "${tmp}/pattern.ppm" <<'EOF'
import sys

W, H = 1024, 768
rows = bytearray()
for y in range(H):
    for x in range(W):
        rows += bytes((x % 256, y % 256, ((x // 256) * 64 + (y // 256) * 32) % 256))
with open(sys.argv[1], "wb") as f:
    f.write(b"P6\n%d %d\n255\n" % (W, H))
    f.write(rows)
EOF

# Deflate-compressed tiled pyramid: the M0 committed master.
mise --cd "${root_dir}/tools/fixtures" exec conda:libvips -- vips tiffsave "${tmp}/pattern.ppm" \
    "${root_dir}/tests/fixtures/rgb_pyramid.tif" \
    --tile --tile-width 256 --tile-height 256 \
    --pyramid --compression deflate

# The same pattern as the other supported source formats:
# lossless tiled JP2 (bit-exact assertions), plain JPEG (lossy,
# tolerance assertions), plain PNG (exact).
# 512px tiles on 1024×768: the bottom row is PARTIAL — deliberately; this
# is the common real-world grid shape and pins region decode on partial
# grids (regression coverage for frames-sg/j2k#62).
mise --cd "${root_dir}/tools/fixtures" exec conda:openjpeg -- opj_compress \
    -i "${tmp}/pattern.ppm" -o "${root_dir}/tests/fixtures/rgb_pyramid.jp2" \
    -t 512,512 -n 4 >/dev/null
# 256px tiles divide 1024×768 exactly: the exact-grid control.
mise --cd "${root_dir}/tools/fixtures" exec conda:openjpeg -- opj_compress \
    -i "${tmp}/pattern.ppm" -o "${root_dir}/tests/fixtures/rgb_exact.jp2" \
    -t 256,256 -n 4 >/dev/null
mise --cd "${root_dir}/tools/fixtures" exec conda:libvips -- vips jpegsave \
    "${tmp}/pattern.ppm" "${root_dir}/tests/fixtures/rgb_plain.jpg" --Q 92
# PNG committed at 512×384 (crop, not resize — keeps pixels exact) so the
# repo does not carry megabytes of fixture.
mise --cd "${root_dir}/tools/fixtures" exec conda:libvips -- vips crop \
    "${tmp}/pattern.ppm" "${tmp}/pattern_small.v" 0 0 512 384
mise --cd "${root_dir}/tools/fixtures" exec conda:libvips -- vips pngsave \
    "${tmp}/pattern_small.v" "${root_dir}/tests/fixtures/rgb_plain.png"

cd "${root_dir}" && shasum -a 256 tests/fixtures/*.tif tests/fixtures/*.jp2 \
    tests/fixtures/*.jpg tests/fixtures/*.png >tests/fixtures/SHA256SUMS
echo "regenerated:"
cat "${root_dir}/tests/fixtures/SHA256SUMS"
