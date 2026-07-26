#!/bin/sh
# Run the official IIIF validator against the Cantaloupe eval image, for
# the side-by-side compliance table in docs/bench/cantaloupe-eval.md.
#
# Same pinned validator and reference image as scripts/validate.sh (run
# that at least once first — it builds the venv and validation.tif).
#
# usage: scripts/validate_cantaloupe.sh [--version 3.0] [--level 2]
set -eu

PORT=18184
IMAGE=${CANTALOUPE_IMAGE:-cantaloupe-eval:openjpeg}
CONF=${CANTALOUPE_CONF:?set CANTALOUPE_CONF to the dir holding the eval properties files}

api_version=""
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
root_dir=$(pwd)
gen=${root_dir}/tests/fixtures/generated
venv="${gen}/validator-venv"
if [ ! -x "${venv}/bin/python" ] || [ ! -f "${gen}/validation.tif" ]; then
  echo "run scripts/validate.sh once first (builds validator venv + reference image)" >&2
  exit 2
fi

docker rm -f validate-cant > /dev/null 2>&1 || true
docker run -d --name validate-cant -m 4g -p "127.0.0.1:${PORT}:8182" \
  -v "${gen}:/imageroot:ro" \
  -v "${CONF}/eval-cache-off.properties:/opt/cantaloupe/cantaloupe.properties:ro" \
  "${IMAGE}" \
  java -Xmx2g -Dcantaloupe.config=/opt/cantaloupe/cantaloupe.properties \
  -jar /opt/cantaloupe/cantaloupe.jar > /dev/null
trap 'docker rm -f validate-cant >/dev/null 2>&1 || true' EXIT
for _ in $(seq 1 180); do
  curl -sf "http://127.0.0.1:${PORT}/health" > /dev/null 2>&1 && break
  sleep 0.5
done

libmagic_dir="$(mise --cd "${root_dir}/tools/fixtures" where conda:libmagic)/lib"
run_suite() {
  suite_version="$1"
  suite_prefix="$2"
  echo "=== Cantaloupe: IIIF Image API ${suite_version}, level ${level}, prefix /${suite_prefix}/ ==="
  DYLD_LIBRARY_PATH="${libmagic_dir}${DYLD_LIBRARY_PATH:+:${DYLD_LIBRARY_PATH}}" \
    LD_LIBRARY_PATH="${libmagic_dir}${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}" \
    "${venv}/bin/python" "${venv}/bin/iiif-validate.py" \
    -s "127.0.0.1:${PORT}" -p "${suite_prefix}" -i "validation.tif" \
    --version "${suite_version}" --level "${level}" -v || true
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
