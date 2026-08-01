#!/bin/sh
# Attach every asset to the draft release, then publish it — in that order,
# and only after the image has been proven and verified.
#
# Draft until now on purpose: immutability is applied when a release is
# published rather than when it is created, so a release made public earlier
# could never receive these assets, and a run that died would have left an
# empty release in public instead of nothing at all.
set -eu

cd "$(dirname "$0")/../.."

: "${DIGEST:?}"
: "${REGISTRY:?}"
: "${IMAGE_NAME:?}"
: "${GITHUB_REF:?}"
: "${GITHUB_REPOSITORY:?}"

tag=${GITHUB_REF#refs/tags/}

# The digest is the thing worth pinning, so put it where people will read it
# rather than making them go and look it up.
cat > pull-instructions.md << INNER

## Container image

\`\`\`console
docker pull ${REGISTRY}/${IMAGE_NAME}:${tag#v}
\`\`\`

Digest-pinned, which is what a deployment should use:

\`\`\`yaml
image: ${REGISTRY}/${IMAGE_NAME}@${DIGEST}
\`\`\`

Verify it came from this workflow at this tag:

\`\`\`console
gh attestation verify oci://${REGISTRY}/${IMAGE_NAME}@${DIGEST} --repo ${GITHUB_REPOSITORY}
\`\`\`

Conformance of this exact image, from the official IIIF validators, is
attached as \`validator-report.txt\`.
INNER

gh release view "${tag}" --json body --jq .body > release-notes.md
cat pull-instructions.md >> release-notes.md
gh release edit "${tag}" --notes-file release-notes.md

gh release upload "${tag}" dist/* validator-report.txt --clobber

# Publish last. Everything above can fail without anything becoming public.
gh release edit "${tag}" --draft=false
echo "published ${tag}"
