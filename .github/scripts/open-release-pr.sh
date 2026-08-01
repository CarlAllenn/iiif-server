#!/bin/sh
# Release phase 1, step 1b: put the prepared bump on a branch and keep exactly
# one Release PR open against it.
#
# The branch is force-pushed on every run so the PR always shows the release
# that would happen if it were merged now, rather than the one that would have
# happened when it was opened.
set -eu

cd "$(dirname "$0")/../.."

: "${VERSION:?VERSION must be set}"
: "${GH_TOKEN:?GH_TOKEN must be set (the release PAT, not GITHUB_TOKEN)}"
: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY must be set}"
branch="release/v${VERSION}"
title="chore: release v${VERSION}"

git config user.name "github-actions[bot]"
git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
git remote set-url origin \
  "https://x-access-token:${GH_TOKEN}@github.com/${GITHUB_REPOSITORY}.git"

git checkout -B "${branch}"
git add Cargo.toml Cargo.lock CHANGELOG.md
# Nothing staged means the branch already matches main — the PR is current.
if git diff --cached --quiet; then
  echo "no changes to propose for v${VERSION}"
  exit 0
fi

# The subject is load-bearing: the tag job keys off exactly this string, and
# so does the guard that stops phase 1 proposing a release on top of a
# release.
git commit -m "${title}"
git push --force origin "${branch}"

body=$(
  cat << EOF
Merging this PR is the commitment point: it tags \`v${VERSION}\` and cuts a
draft GitHub release, which triggers the publish workflow to build, sign and
attach the container image and binaries.

Nothing is published until this is merged, and nothing outside this path
publishes at all.

- Version bumped to \`${VERSION}\` across the workspace and its internal
  dependency constraints
- \`CHANGELOG.md\` regenerated from the conventional commits since the last tag

Review the changelog as release notes: this text is what the GitHub release
will carry.
EOF
)

existing=$(gh pr list --head "${branch}" --state open --json number --jq '.[0].number // empty')
if [ -n "${existing}" ]; then
  gh pr edit "${existing}" --title "${title}" --body "${body}"
  echo "updated PR #${existing}"
else
  gh pr create --head "${branch}" --base main --title "${title}" --body "${body}"
  echo "opened the release PR for v${VERSION}"
fi
