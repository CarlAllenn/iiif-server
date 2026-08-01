#!/bin/sh
# Release phase 1, step 2: the Release PR has been merged, so tag it and cut
# the draft release. Publishes nothing — pushing the tag is what starts
# phase 2 (publish.yml), which builds, signs and fills the draft.
#
# The tag MUST be pushed with a PAT. Tags pushed with the default GITHUB_TOKEN
# do not trigger workflows (GitHub's recursion guard), and a release that
# silently triggers nothing looks exactly like a success.
set -eu

cd "$(dirname "$0")/../.."

version=$(taplo get -f Cargo.toml 'workspace.package.version')
tag="v${version}"

# Guard: only ever tag a commit that is a release commit. A workflow_dispatch
# on an ordinary commit would otherwise mint a tag for a version whose
# manifests and changelog were never prepared.
subject=$(git log -1 --pretty=%s)
case "${subject}" in
  "chore: release ${tag}"*) ;;
  *)
    echo "FAIL: HEAD is not the release commit for ${tag}" >&2
    echo "  subject: ${subject}" >&2
    exit 1
    ;;
esac

if git rev-parse -q --verify "refs/tags/${tag}" > /dev/null; then
  echo "${tag} already exists; nothing to do"
  exit 0
fi

# Annotated, so the tag carries an author and a date of its own.
git tag -a "${tag}" -m "${tag}"
git push origin "${tag}"
echo "pushed ${tag}"

# Release notes are the changelog section for this version — the same text
# reviewers already approved in the Release PR, not a second description of
# it written by a machine at a different time.
notes=$(git cliff --latest --strip all)

# Draft, always. Immutability applies when a release is published rather than
# when it is created, so a release made public now could never receive the
# assets phase 2 attaches; and a phase 2 that dies leaves nothing public
# instead of an empty release.
printf '%s\n' "${notes}" | gh release create "${tag}" \
  --draft \
  --title "${tag}" \
  --notes-file -
echo "created draft release ${tag}"
