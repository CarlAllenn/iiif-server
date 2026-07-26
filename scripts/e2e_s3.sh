#!/bin/sh
# End-to-end object-store serving: MinIO in Docker, fixtures uploaded via
# mc, the real server pointed at s3://…, and pixel-bearing requests
# verified over HTTP. Point the env at Hetzner Object Storage to run the
# same checks against the real target.
set -eu

cd "$(dirname "$0")/.."
root_dir=$(pwd)

CONTAINER=iiif-e2e-minio
PORT=9200
SERVER_PORT=6868
export AWS_ACCESS_KEY_ID=minioadmin
export AWS_SECRET_ACCESS_KEY=minioadmin
export AWS_REGION=us-east-1

docker rm -f "${CONTAINER}" > /dev/null 2>&1 || true
docker run -d --name "${CONTAINER}" -p "${PORT}:9000" \
  minio/minio server /data > /dev/null
server_pid=0
trap 'docker rm -f "${CONTAINER}" >/dev/null 2>&1 || true; kill "${server_pid}" 2>/dev/null || true' EXIT

for _ in $(seq 1 60); do
  if curl -sf "http://127.0.0.1:${PORT}/minio/health/ready" > /dev/null 2>&1; then
    break
  fi
  sleep 0.25
done

mc() {
  mise --cd "${root_dir}/tools/fixtures" exec aqua:minio/mc -- mc "$@"
}
mc --quiet alias set e2eminio "http://127.0.0.1:${PORT}" minioadmin minioadmin > /dev/null
mc --quiet mb e2eminio/masters > /dev/null
mc --quiet cp "${root_dir}/tests/fixtures/rgb_pyramid.tif" \
  "${root_dir}/tests/fixtures/rgb_pyramid.jp2" \
  e2eminio/masters/collection/ > /dev/null

cargo build --release -p iiif-server > /dev/null
./target/release/iiif-server serve s3://masters/collection \
  --endpoint "http://127.0.0.1:${PORT}" \
  --bind "127.0.0.1:${SERVER_PORT}" &
server_pid=$!
for _ in $(seq 1 50); do
  if curl -sf "http://127.0.0.1:${SERVER_PORT}/healthz" > /dev/null 2>&1; then
    break
  fi
  sleep 0.1
done

fail=0
check() {
  url=$1
  expect=$2
  got=$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:${SERVER_PORT}${url}")
  if [ "${got}" = "${expect}" ]; then
    echo "ok   ${got} ${url}"
  else
    echo "FAIL ${got} (want ${expect}) ${url}"
    fail=1
  fi
}

check "/iiif/3/rgb_pyramid.tif/info.json" 200
check "/iiif/3/rgb_pyramid.tif/256,256,256,256/128,/0/default.jpg" 200
check "/iiif/3/rgb_pyramid.jp2/info.json" 200
check "/iiif/3/rgb_pyramid.jp2/300,200,256,256/max/0/default.png" 200
check "/iiif/2/rgb_pyramid.jp2/full/full/0/default.jpg" 200
check "/iiif/3/missing.tif/info.json" 404

# Pixel spot-check through the whole stack: object store → decode → PNG.
curl -s "http://127.0.0.1:${SERVER_PORT}/iiif/3/rgb_pyramid.jp2/300,200,8,8/max/0/default.png" \
  -o /tmp/e2e_s3_probe.png
uv run --quiet --with pillow python - << 'EOF'
from PIL import Image

img = Image.open("/tmp/e2e_s3_probe.png").convert("RGB")
got = img.getpixel((0, 0))
expected = (300 % 256, 200 % 256, ((300 // 256) * 64 + (200 // 256) * 32) % 256)
assert got == expected, f"pixel mismatch: {got} != {expected}"
print(f"ok   pixel {got} matches pattern")
EOF

exit "${fail}"
