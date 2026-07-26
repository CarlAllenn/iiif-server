#!/bin/sh
# Run the official IIIF validator against a locally built server.
#
# The validator is pinned from the IIIF/image-validator git repo (the PyPI
# package is stale, 2019) and executed via uv. The reference image (the
# validator's colored-squares master) is fetched at the same pinned commit,
# digest-verified, and converted to a pyramidal TIFF for serving —
# dev-time fixture tooling, exempt from the zero-C doctrine.
#
# usage: scripts/validate.sh [--version 3.0] [--level 2]
set -eu

VALIDATOR_SHA=1740893f1fb22960142071a9f3d1c99122a190c7
REF_NAME=67352ccc-d1b0-11e1-89ae-279075081939
REF_SHA256=c67abb4dc9650b4d69b46a4ef0453428ea860d63b02ac406d3e0d7425167d736
PORT=6464

api_version="3.0"
level=2
while [ $# -gt 0 ]; do
    case "$1" in
    --version)
        api_version="$2"
        shift 2
        ;;
    --level)
        level="$2"
        shift 2
        ;;
    *)
        echo "unknown flag: $1" >&2
        exit 2
        ;;
    esac
done

cd "$(dirname "$0")/.."
gen=tests/fixtures/generated
mkdir -p "${gen}"

# 1. Reference image, digest-verified.
if [ ! -f "${gen}/${REF_NAME}.png" ]; then
    curl -sSfL \
        "https://raw.githubusercontent.com/IIIF/image-validator/${VALIDATOR_SHA}/html/${REF_NAME}.png" \
        -o "${gen}/${REF_NAME}.png.tmp"
    echo "${REF_SHA256}  ${gen}/${REF_NAME}.png.tmp" | shasum -a 256 -c - >/dev/null
    mv "${gen}/${REF_NAME}.png.tmp" "${gen}/${REF_NAME}.png"
fi

# 2. Convert to the pyramidal TIFF the server serves.
if [ ! -f "${gen}/validation.tif" ]; then
    mise exec conda:libvips -- vips tiffsave \
        "${gen}/${REF_NAME}.png" "${gen}/validation.tif" \
        --tile --tile-width 256 --tile-height 256 \
        --pyramid --compression deflate
fi

# 3. Build and start the server.
cargo build --release -p iiif-server >/dev/null
./target/release/iiif-server serve "${gen}" --bind "127.0.0.1:${PORT}" &
server_pid=$!
trap 'kill "${server_pid}" 2>/dev/null || true' EXIT
for _ in $(seq 1 50); do
    if curl -sf "http://127.0.0.1:${PORT}/healthz" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done

# 4. Validate. Exit code is the number of failed tests.
#
# python-magic needs libmagic (conda-provided, via DYLD/LD paths). The
# console script's own `#!/bin/sh` shebang would strip DYLD_* through
# macOS SIP, so we run the interpreter directly on the script instead of
# exec'ing the script.
libmagic_dir="$(mise where conda:libmagic)/lib"
venv="${gen}/validator-venv"
if [ ! -x "${venv}/bin/python" ]; then
    uv venv --quiet "${venv}"
    uv pip install --quiet --python "${venv}/bin/python" \
        "iiif-validator @ git+https://github.com/IIIF/image-validator@${VALIDATOR_SHA}"
fi
DYLD_LIBRARY_PATH="${libmagic_dir}${DYLD_LIBRARY_PATH:+:${DYLD_LIBRARY_PATH}}" \
    LD_LIBRARY_PATH="${libmagic_dir}${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}" \
    "${venv}/bin/python" "${venv}/bin/iiif-validate.py" \
    -s "127.0.0.1:${PORT}" -p "iiif/3" -i "validation.tif" \
    --version "${api_version}" --level "${level}" -v
