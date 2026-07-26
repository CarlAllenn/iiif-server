#!/bin/sh
# Corpus for the Cantaloupe head-to-head eval (docs/bench/cantaloupe-eval.md).
#
# Fully synthetic, mirroring common digitization profiles (issue #1): a
# typical scan at 6500×4300 whose 1024px tile grid does NOT divide the
# dimensions (the partial-grid fallback path, frames-sg/j2k#62), an
# exact-grid control at 6144×4096, an untiled-with-precincts codestream,
# a large master at 15000×11000, and HTJ2K variants encoded with OpenJPH
# — an encoder independent of both servers' decoders.
#
# Dev-time fixture generation is exempt from the zero-C doctrine: libvips,
# OpenJPEG, and OpenJPH run here, never in the product.
set -eu

cd "$(dirname "$0")/.."
root_dir=$(pwd)
gen=${root_dir}/tests/fixtures/generated/eval
mkdir -p "${gen}"
tmp=$(mktemp -d)
trap 'rm -rf "${tmp}"' EXIT

vips() {
    mise --cd "${root_dir}/tools/fixtures" exec conda:libvips -- vips "$@"
}
opj() {
    mise --cd "${root_dir}/tools/fixtures" exec conda:openjpeg -- opj_compress "$@"
}
ojph() {
    mise --cd "${root_dir}/tools/fixtures" exec 'conda:openjph@0.30.1' -- ojph_compress "$@"
}

# Smooth photographic-ish pattern (same family as the spike fixtures):
# JPEG/wavelet-friendly entropy, position-identifiable content.
pattern() { # pattern WIDTH HEIGHT OUT.ppm
    uv run --quiet --with numpy python - "$1" "$2" "$3" <<'EOF'
import sys

import numpy as np

W, H = int(sys.argv[1]), int(sys.argv[2])
x = np.arange(W, dtype=np.float64)
y = np.arange(H, dtype=np.float64)[:, None]
img = np.empty((H, W, 3), dtype=np.uint8)
img[..., 0] = np.broadcast_to(127.5 + 127.5 * np.sin(x / 211.0), (H, W)).round()
img[..., 1] = np.broadcast_to(127.5 + 127.5 * np.sin(y / 173.0), (H, W)).round()
img[..., 2] = (127.5 + 127.5 * np.sin((x + y) / 101.0)).round()
with open(sys.argv[3], "wb") as f:
    f.write(b"P6\n%d %d\n255\n" % (W, H))
    f.write(img.tobytes())
EOF
}

done_all=true
for f in scan_partial_ll.jp2 scan_partial_r20.jp2 scan_untiled_ll.jp2 \
    scan_ht_ll.j2c scan_ht_lossy.j2c scan_pyr_deflate.tif \
    scan_pyr_jpeg.tif scan_plain.jpg exact_ll.jp2 exact_ht_ll.j2c \
    large_partial_ll.jp2 large_ht_ll.j2c; do
    [ -f "${gen}/${f}" ] || done_all=false
done
if [ "${done_all}" = true ]; then
    echo "eval corpus already present in ${gen}"
    exit 0
fi

# --- Typical scan, 6500×4300: 1024 does not divide either dimension. ---
pattern 6500 4300 "${tmp}/scan.ppm"

opj -i "${tmp}/scan.ppm" -o "${gen}/scan_partial_ll.jp2" \
    -t 1024,1024 -n 6 >/dev/null
opj -i "${tmp}/scan.ppm" -o "${gen}/scan_partial_r20.jp2" \
    -t 1024,1024 -n 6 -r 20 -I >/dev/null
# Untiled codestream with 256px precincts: the common kdu/opj single-tile
# profile.
opj -i "${tmp}/scan.ppm" -o "${gen}/scan_untiled_ll.jp2" \
    -c '[256,256]' -n 6 >/dev/null
# HTJ2K (Part 15), raw codestreams from OpenJPH.
ojph -i "${tmp}/scan.ppm" -o "${gen}/scan_ht_ll.j2c" \
    -tile_size '{1024,1024}' -num_decomps 6 -reversible true >/dev/null
ojph -i "${tmp}/scan.ppm" -o "${gen}/scan_ht_lossy.j2c" \
    -tile_size '{1024,1024}' -num_decomps 6 -reversible false >/dev/null
# The same master as the TIFF profiles digitization actually produces,
# plus the small-collection plain-JPEG case.
vips tiffsave "${tmp}/scan.ppm" "${gen}/scan_pyr_deflate.tif" \
    --tile --tile-width 256 --tile-height 256 --pyramid \
    --compression deflate
vips tiffsave "${tmp}/scan.ppm" "${gen}/scan_pyr_jpeg.tif" \
    --tile --tile-width 256 --tile-height 256 --pyramid \
    --compression jpeg --Q 90
vips jpegsave "${tmp}/scan.ppm" "${gen}/scan_plain.jpg" --Q 92
rm -f "${tmp}/scan.ppm"

# --- Exact-grid control, 6144×4096: 1024 divides both dimensions. ---
pattern 6144 4096 "${tmp}/exact.ppm"
opj -i "${tmp}/exact.ppm" -o "${gen}/exact_ll.jp2" \
    -t 1024,1024 -n 6 >/dev/null
ojph -i "${tmp}/exact.ppm" -o "${gen}/exact_ht_ll.j2c" \
    -tile_size '{1024,1024}' -num_decomps 6 -reversible true >/dev/null
rm -f "${tmp}/exact.ppm"

# --- Large master, 15000×11000 (165 MP): partial grid. ---
pattern 15000 11000 "${tmp}/large.ppm"
opj -i "${tmp}/large.ppm" -o "${gen}/large_partial_ll.jp2" \
    -t 1024,1024 -n 7 >/dev/null
ojph -i "${tmp}/large.ppm" -o "${gen}/large_ht_ll.j2c" \
    -tile_size '{1024,1024}' -num_decomps 7 -reversible true >/dev/null
rm -f "${tmp}/large.ppm"

echo "eval corpus:"
ls -la "${gen}"
