#!/bin/sh
# Verify the published image the way a stranger would, and specifically that
# the signature and provenance name THIS TAG.
#
# This is the step the whole two-phase split exists to make possible. edtf
# shipped v1.0.0 attestations naming a commit that built none of the
# published bytes, and only noticed afterwards — by which point Sigstore's
# append-only log had made it permanent. Checking here means a wrong
# attestation fails the release instead of outliving it.
set -eu

: "${REGISTRY:?}"
: "${IMAGE_NAME:?}"
: "${DIGEST:?}"
: "${GITHUB_REF:?}"
: "${GITHUB_SERVER_URL:?}"
: "${GITHUB_REPOSITORY:?}"

image="${REGISTRY}/${IMAGE_NAME}@${DIGEST}"
identity="${GITHUB_SERVER_URL}/${GITHUB_REPOSITORY}/.github/workflows/publish.yml@${GITHUB_REF}"

echo "--- cosign: signed by ${identity}"
cosign verify \
  --certificate-identity "${identity}" \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  "${image}" > /dev/null
echo "    ok"

echo "--- gh attestation: provenance for ${DIGEST}"
gh attestation verify "oci://${image}" \
  --repo "${GITHUB_REPOSITORY}" \
  --source-ref "${GITHUB_REF}"
echo "    ok"
