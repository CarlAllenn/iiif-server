#!/bin/sh
# Generate the `curl | sh` installer for this release and write it to
# dist/install.sh, with the version and every checksum baked in.
#
# Generated rather than committed because a static installer would have to
# resolve "latest" at run time, which means trusting whatever the API returns
# at that moment. Baking the version and the checksums in means the script a
# user pipes into their shell can only install the exact bytes this release
# published, and says so if it cannot.
#
# dist (cargo-dist) would normally own this. It cannot here: it would have to
# build the binaries itself to name them, and building *-linux-musl on a
# runner needs a C toolchain for mimalloc that harden-runner's disable-sudo
# forbids — see docs/release-engineering.md.
set -eu

cd "$(dirname "$0")/../.."

: "${GITHUB_REF:?GITHUB_REF must be set (this runs on a tag)}"
: "${GITHUB_REPOSITORY:?}"

tag=${GITHUB_REF#refs/tags/}
base="https://github.com/${GITHUB_REPOSITORY}/releases/download/${tag}"

# Build the target -> checksum table from the artifacts actually present, so
# a target that failed to build cannot silently become an installer entry
# that 404s.
table=""
for sum in dist/*.tar.gz.sha256; do
  [ -e "${sum}" ] || continue
  file=$(basename "${sum}" .sha256)
  # Both sha256sum (Linux) and shasum (macOS) print "<hash>  <name>".
  hash=$(cut -d' ' -f1 < "${sum}")
  target=$(echo "${file}" | sed -e "s|^iiif-server-${tag}-||" -e 's|\.tar\.gz$||')
  table="${table}${target} ${hash} ${file}
"
done

if [ -z "${table}" ]; then
  echo "FAIL: no built archives found in dist/" >&2
  exit 1
fi

cat > dist/install.sh << INSTALLER
#!/bin/sh
# iiif-server ${tag} installer.
#
# Downloads the binary for this platform from the GitHub release, verifies its
# SHA-256 against a value baked in at release time, and installs it.
#
#   curl -LsSf ${base}/install.sh | sh
#
# Set IIIF_INSTALL_DIR to choose where it lands (default: \$HOME/.local/bin).
#
# Prefer to verify provenance rather than just integrity? Every release is
# signed; see SECURITY.md for the cosign and gh attestation invocations.
set -eu

TAG="${tag}"
BASE="${base}"
INSTALL_DIR="\${IIIF_INSTALL_DIR:-\$HOME/.local/bin}"

# target  sha256  filename
CHECKSUMS="${table}"

fail() {
  echo "install: \$*" >&2
  exit 1
}

os=\$(uname -s)
arch=\$(uname -m)

case "\${os}/\${arch}" in
  Linux/x86_64 | Linux/amd64) target=x86_64-unknown-linux-musl ;;
  Linux/aarch64 | Linux/arm64) target=aarch64-unknown-linux-musl ;;
  Darwin/arm64) target=aarch64-apple-darwin ;;
  Darwin/x86_64) target=x86_64-apple-darwin ;;
  *)
    fail "no prebuilt binary for \${os}/\${arch}.
  Linux (x86_64/aarch64) and macOS (Apple Silicon/Intel) are published.
  Windows is not yet: see the tracking issue in the README.
  Otherwise build from source — the repository has the toolchain pinned."
    ;;
esac

entry=\$(echo "\${CHECKSUMS}" | grep "^\${target} " || true)
[ -n "\${entry}" ] || fail "release \${TAG} published nothing for \${target}"
expected=\$(echo "\${entry}" | cut -d' ' -f2)
file=\$(echo "\${entry}" | cut -d' ' -f3)

tmp=\$(mktemp -d)
trap 'rm -rf "\${tmp}"' EXIT

echo "iiif-server \${TAG} (\${target})"
echo "  downloading"
if command -v curl > /dev/null 2>&1; then
  curl -LsSf "\${BASE}/\${file}" -o "\${tmp}/\${file}"
elif command -v wget > /dev/null 2>&1; then
  wget -qO "\${tmp}/\${file}" "\${BASE}/\${file}"
else
  fail "neither curl nor wget is available"
fi

echo "  verifying"
if command -v sha256sum > /dev/null 2>&1; then
  actual=\$(sha256sum "\${tmp}/\${file}" | cut -d' ' -f1)
elif command -v shasum > /dev/null 2>&1; then
  actual=\$(shasum -a 256 "\${tmp}/\${file}" | cut -d' ' -f1)
else
  fail "no sha256 tool available; refusing to install unverified bytes"
fi
[ "\${actual}" = "\${expected}" ] || fail "checksum mismatch for \${file}
  expected \${expected}
  actual   \${actual}
  Do not use these bytes."

tar -xzf "\${tmp}/\${file}" -C "\${tmp}"
mkdir -p "\${INSTALL_DIR}"
install -m 0755 "\${tmp}/iiif-server" "\${INSTALL_DIR}/iiif-server"

echo "  installed \${INSTALL_DIR}/iiif-server"
case ":\${PATH}:" in
  *":\${INSTALL_DIR}:"*) ;;
  *) echo "  note: \${INSTALL_DIR} is not on your PATH" ;;
esac
"\${INSTALL_DIR}/iiif-server" --version
INSTALLER

chmod +x dist/install.sh
echo "generated dist/install.sh for ${tag}"
