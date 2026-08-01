#!/bin/sh
# Release phase 1, step 1: work out the next version and stage everything the
# Release PR should contain.
#
# Writes GITHUB_OUTPUT keys `version` (bare, e.g. 0.2.0) and `release` (true
# when there is something to release). Leaves the working tree modified; the
# caller commits it.
#
# The version is derived, never typed: git cliff reads the conventional
# commits since the last v* tag. See cliff.toml for the two decisions that
# matter — 0.x breaking changes bump the minor rather than reaching 1.0.0,
# and chore/ci/docs-only ranges release nothing.
set -eu

cd "$(dirname "$0")/../.."

current=$(taplo get -f Cargo.toml 'workspace.package.version')
next=$(git cliff --bumped-version)
version=${next#v}

echo "current: ${current}"
echo "next:    ${version}"

emit() {
  if [ -n "${GITHUB_OUTPUT:-}" ]; then
    echo "$1" >> "${GITHUB_OUTPUT}"
  fi
}

if [ "${version}" = "${current}" ]; then
  echo "nothing to release: no version-bumping commits since the last tag"
  emit "release=false"
  exit 0
fi

# Three places hold the version, and cargo refuses to resolve if they
# disagree: the workspace package version that every crate inherits, and the
# two internal dependency constraints in [workspace.dependencies]. Keeping
# them in lockstep is exactly what release-plz could not do here.
sed -i.bak "s|^version = \"${current}\"\$|version = \"${version}\"|" Cargo.toml
sed -i.bak "s|\(iiif-core = { path = \"crates/core\", version = \)\"${current}\"|\1\"${version}\"|" Cargo.toml
sed -i.bak "s|\(iiif-sources = { path = \"crates/sources\", version = \)\"${current}\"|\1\"${version}\"|" Cargo.toml
rm -f Cargo.toml.bak

# Fail loudly rather than open a PR that does not build. A stale occurrence
# means one of the substitutions above missed, which is how a workspace ends
# up with `iiif-core = "^0.1.0"` pointing at a 0.2.0 crate.
if grep -q "\"${current}\"" Cargo.toml; then
  echo "FAIL: Cargo.toml still mentions ${current} after the bump:" >&2
  grep -n "\"${current}\"" Cargo.toml >&2
  exit 1
fi

# Refresh the lockfile's own copy of the workspace member versions.
cargo update --workspace --offline 2> /dev/null || cargo update --workspace

# Prove the tree still resolves before anyone is asked to review it.
cargo metadata --format-version 1 --no-deps > /dev/null

git cliff --bump --output CHANGELOG.md

# git-cliff separates releases with a trailing blank line, which at end of
# file is an MD012/MD047 violation — and every markdown file in this repo is
# linted with warnings as errors. Collapse to exactly one final newline.
changelog=$(cat CHANGELOG.md)
printf '%s\n' "${changelog}" > CHANGELOG.md

emit "release=true"
emit "version=${version}"
echo "prepared ${version}"
