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

# By default both API versions run (3.0 then 2.0); --version picks one.
api_version=""
level=2
# --server HOST:PORT validates something already running instead of building
# and starting one here. That is how the release pipeline points the official
# validators at the *published container image* rather than at an equivalent
# local build: same source, but a musl static binary through cargo-auditable
# inside a scratch image is not the same artifact as a host cargo build, and
# the conformance claim should cover the bytes people actually pull.
external_server=""
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
    --server)
      external_server="$2"
      shift 2
      ;;
    *)
      echo "unknown flag: $1" >&2
      exit 2
      ;;
  esac
done

cd "$(dirname "$0")/.."
root_dir=$(pwd)
# Fixture tools (libvips, libmagic) — mise exec auto-installs, but the
# mise-where lookup below does not, so install explicitly (idempotent).
mise --cd "${root_dir}/tools/fixtures" install --quiet
gen=${root_dir}/tests/fixtures/generated
mkdir -p "${gen}"

# 1. Reference image, digest-verified.
if [ ! -f "${gen}/${REF_NAME}.png" ]; then
  curl -sSfL \
    "https://raw.githubusercontent.com/IIIF/image-validator/${VALIDATOR_SHA}/html/${REF_NAME}.png" \
    -o "${gen}/${REF_NAME}.png.tmp"
  echo "${REF_SHA256}  ${gen}/${REF_NAME}.png.tmp" | shasum -a 256 -c - > /dev/null
  mv "${gen}/${REF_NAME}.png.tmp" "${gen}/${REF_NAME}.png"
fi

# 2. Convert to the pyramidal TIFF the server serves.
if [ ! -f "${gen}/validation.tif" ]; then
  mise --cd "${root_dir}/tools/fixtures" exec conda:libvips -- vips tiffsave \
    "${gen}/${REF_NAME}.png" "${gen}/validation.tif" \
    --tile --tile-width 256 --tile-height 256 \
    --pyramid --compression deflate
fi

# 3. Build and start the server, unless one was handed to us.
if [ -n "${external_server}" ]; then
  server="${external_server}"
else
  server="127.0.0.1:${PORT}"
  cargo build --release -p iiif-server > /dev/null
  ./target/release/iiif-server serve "${gen}" --bind "${server}" &
  server_pid=$!
  trap 'kill "${server_pid}" 2>/dev/null || true' EXIT
fi
for _ in $(seq 1 50); do
  if curl -sf "http://${server}/healthz" > /dev/null 2>&1; then
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
libmagic_dir="$(mise --cd "${root_dir}/tools/fixtures" where conda:libmagic)/lib"
venv="${gen}/validator-venv"
if [ ! -x "${venv}/bin/python" ]; then
  uv venv --quiet "${venv}"
  uv pip install --quiet --python "${venv}/bin/python" \
    "iiif-validator @ git+https://github.com/IIIF/image-validator@${VALIDATOR_SHA}"
fi

run_suite() {
  suite_version="$1"
  suite_prefix="$2"
  echo "=== IIIF Image API ${suite_version}, level ${level}, prefix /${suite_prefix}/ ==="
  DYLD_LIBRARY_PATH="${libmagic_dir}${DYLD_LIBRARY_PATH:+:${DYLD_LIBRARY_PATH}}" \
    LD_LIBRARY_PATH="${libmagic_dir}${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}" \
    "${venv}/bin/python" "${venv}/bin/iiif-validate.py" \
    -s "${server}" -p "${suite_prefix}" -i "validation.tif" \
    --version "${suite_version}" --level "${level}" -v
}

case "${api_version}" in
  "")
    run_suite 3.0 iiif/3
    run_suite 2.0 iiif/2
    ;;
  3.0) run_suite 3.0 iiif/3 ;;
  2.0 | 2.1) run_suite "${api_version}" iiif/2 ;;
  *)
    echo "unsupported --version ${api_version}" >&2
    exit 2
    ;;
esac
