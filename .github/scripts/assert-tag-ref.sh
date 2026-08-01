#!/bin/sh
# Refuse to publish from anything that is not a version tag, and refuse to
# publish a tag that disagrees with the manifest.
#
# Provenance is the product here. A run dispatched from a branch would sign
# with `refs/heads/...` provenance — the exact defect the two-phase split
# exists to prevent, and one that cannot be corrected afterwards because
# Sigstore is append-only.
set -eu

cd "$(dirname "$0")/../.."

case "${GITHUB_REF:-}" in
  refs/tags/v*) ;;
  *)
    echo "FAIL: refusing to publish from '${GITHUB_REF:-<unset>}'" >&2
    echo "  publish.yml must run on a v* tag; dispatch with --ref <tag>." >&2
    exit 1
    ;;
esac

tag_version=${GITHUB_REF#refs/tags/v}
# Read the workspace version without needing a TOML tool: it is the first
# bare `version = "..."` line, under [workspace.package].
manifest_version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)

if [ "${tag_version}" != "${manifest_version}" ]; then
  echo "FAIL: tag v${tag_version} does not match the manifest (${manifest_version})" >&2
  echo "  a tag that names a version the tree does not contain would sign the wrong bytes." >&2
  exit 1
fi

echo "publishing v${tag_version} from ${GITHUB_REF}"
