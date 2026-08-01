#!/bin/sh
# Prove a built container image actually serves IIIF before it is published.
#
# Not a conformance check — scripts/validate.sh runs the official validators,
# and the release pipeline points those at the image too. This is the cheaper
# gate that runs per architecture: does the thing start, serve real pixels,
# report its version, answer its own healthcheck, and stop on SIGTERM without
# being killed. A published image that fails any of those is worse than no
# image at all.
#
# usage: scripts/smoke_image.sh [IMAGE_REF] [PORT]
set -eu

image=${1:-iiif-server:local}
port=${2:-6403}
container=iiif-smoke-$$

cd "$(dirname "$0")/.."
root_dir=$(pwd)

cleanup() {
  docker rm -f "${container}" > /dev/null 2>&1 || true
}
trap cleanup EXIT

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

echo "--- ${image}: --version through the entrypoint"
version=$(docker run --rm "${image}" --version) || fail "--version exited non-zero"
case "${version}" in
  "iiif-server "*) echo "    ${version}" ;;
  *) fail "unexpected --version output: ${version}" ;;
esac

# Hardened exactly as docs/deployment.md tells operators to run it: read-only
# root, no capabilities, no privilege escalation. If the image cannot serve
# under those flags the recipe is a lie.
echo "--- starting container (read-only, no capabilities)"
docker run -d --name "${container}" -p "${port}:6363" \
  -v "${root_dir}/tests/fixtures:/imageroot:ro" \
  --read-only --cap-drop ALL --security-opt no-new-privileges \
  "${image}" > /dev/null

for _ in $(seq 1 50); do
  if curl -sf "http://127.0.0.1:${port}/healthz" > /dev/null 2>&1; then
    break
  fi
  sleep 0.2
done
curl -sf "http://127.0.0.1:${port}/healthz" > /dev/null || fail "/healthz never answered"
echo "    /healthz ok"

echo "--- info.json"
info=$(curl -sf "http://127.0.0.1:${port}/iiif/3/rgb_pyramid.tif/info.json") || fail "info.json request failed"
echo "${info}" | grep -q '"profile":"level2"' || fail "info.json missing level2 profile"
echo "${info}" | grep -q '"width":1024' || fail "info.json has the wrong width"
echo "    level2, 1024px wide"

echo "--- derivative"
status=$(curl -s -o /tmp/smoke-derivative.jpg -w '%{http_code}:%{content_type}:%{size_download}' \
  "http://127.0.0.1:${port}/iiif/3/rgb_pyramid.tif/full/200,/0/default.jpg")
case "${status}" in
  200:image/jpeg:*) ;;
  *) fail "derivative returned ${status}" ;;
esac
bytes=${status##*:}
[ "${bytes}" -gt 1000 ] || fail "derivative suspiciously small (${bytes} bytes)"
echo "    200 image/jpeg, ${bytes} bytes"

echo "--- container healthcheck reaches healthy"
health=unknown
for _ in $(seq 1 30); do
  health=$(docker inspect --format '{{.State.Health.Status}}' "${container}" 2> /dev/null || echo unknown)
  [ "${health}" = "healthy" ] && break
  sleep 1
done
[ "${health}" = "healthy" ] || fail "healthcheck never reported healthy (last: ${health})"
echo "    healthy"

# The regression this guards: with no SIGTERM handler a container's PID 1
# ignores the signal outright, so `docker stop` burns its whole timeout and
# exits 137 (SIGKILL). A clean exit here means in-flight requests drain.
echo "--- graceful stop"
docker stop "${container}" > /dev/null
exit_code=$(docker inspect --format '{{.State.ExitCode}}' "${container}")
[ "${exit_code}" = "0" ] || fail "container exited ${exit_code} (137 means it was killed, not stopped)"
echo "    exited 0"

rm -f /tmp/smoke-derivative.jpg
echo "PASS: ${image}"
