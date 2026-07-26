#!/bin/sh
# Regenerate the committed test masters in tests/fixtures/.
#
# Dev-time fixture generation is exempt from the zero-C doctrine (design
# spec, Quality regime): libvips runs here, never in the product. The
# pattern is deterministic — every pixel encodes its own coordinates
# (r = x mod 256, g = y mod 256, b marks the 256px block) — so tests can
# assert exact pixel values at any position.
#
# Requires: python3, and libvips via `mise exec conda:libvips@latest`.
set -eu

cd "$(dirname "$0")/.."
mkdir -p tests/fixtures
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
mise exec conda:libvips@latest -- vips tiffsave "${tmp}/pattern.ppm" \
    tests/fixtures/rgb_pyramid.tif \
    --tile --tile-width 256 --tile-height 256 \
    --pyramid --compression deflate

shasum -a 256 tests/fixtures/*.tif >tests/fixtures/SHA256SUMS
echo "regenerated:"
cat tests/fixtures/SHA256SUMS
