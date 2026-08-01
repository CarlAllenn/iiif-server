#!/bin/sh
# Assemble the per-architecture digests into one multi-architecture manifest
# and apply the tag ladder, then emit the manifest digest.
#
# One tag resolves to amd64 or arm64 depending on who pulls it; nobody has to
# think about it. The digest this prints is what everything downstream signs,
# verifies and publishes — never a tag, which is a moving name.
set -eu

cd "$(dirname "$0")/../.."

: "${REGISTRY:?}"
: "${IMAGE_NAME:?}"
: "${TAGS:?TAGS must be set (docker/metadata-action output)}"

tag_args=""
for tag in ${TAGS}; do
  tag_args="${tag_args} --tag ${tag}"
done

digest_args=""
for file in /tmp/digests/*; do
  digest_args="${digest_args} ${REGISTRY}/${IMAGE_NAME}@sha256:$(basename "${file}")"
done

# shellcheck disable=SC2086 # both lists are deliberately word-split
docker buildx imagetools create ${tag_args} ${digest_args}

# Resolve the tag we just wrote back to its manifest-list digest.
first_tag=$(echo "${TAGS}" | head -1)
digest=$(docker buildx imagetools inspect "${first_tag}" --format '{{.Manifest.Digest}}')

echo "manifest digest: ${digest}"
if [ -n "${GITHUB_OUTPUT:-}" ]; then
  echo "digest=${digest}" >> "${GITHUB_OUTPUT}"
fi
