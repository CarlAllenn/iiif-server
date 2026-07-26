#!/bin/sh
# M0 object-store mini-spike runner: MinIO in Docker, bucket via mc, then
# the measurement example. Point SPIKE_ENDPOINT etc. at Hetzner Object
# Storage instead to reproduce the numbers against the real target.
set -eu

cd "$(dirname "$0")/.."

CONTAINER=iiif-spike-minio
PORT=9100
export SPIKE_ENDPOINT="http://127.0.0.1:${PORT}"
export SPIKE_BUCKET=spike
export AWS_ACCESS_KEY_ID=minioadmin
export AWS_SECRET_ACCESS_KEY=minioadmin
export SPIKE_OBJECT=tests/fixtures/generated/spike2_lossless.jp2

if [ ! -f "${SPIKE_OBJECT}" ]; then
  echo "run scripts/gen_spike2.sh first (needs the 17 MB JP2 master)" >&2
  exit 2
fi

docker rm -f "${CONTAINER}" > /dev/null 2>&1 || true
docker run -d --name "${CONTAINER}" -p "${PORT}:9000" \
  minio/minio server /data > /dev/null
trap 'docker rm -f "${CONTAINER}" >/dev/null 2>&1 || true' EXIT

# Wait for MinIO, then create the bucket.
for _ in $(seq 1 60); do
  if curl -sf "http://127.0.0.1:${PORT}/minio/health/ready" > /dev/null 2>&1; then
    break
  fi
  sleep 0.25
done
mise exec aqua:minio/mc -- mc --quiet alias set spikeminio \
  "http://127.0.0.1:${PORT}" minioadmin minioadmin > /dev/null
mise exec aqua:minio/mc -- mc --quiet mb "spikeminio/${SPIKE_BUCKET}" > /dev/null

cargo run --release -p iiif-sources --example objstore_spike
