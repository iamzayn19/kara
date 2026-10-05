#!/bin/sh
# Install the latest (or a given) Veyra release from GitHub Releases.
#   curl -fsSL https://raw.githubusercontent.com/iamzayn19/veyra/main/scripts/install.sh | sh
#   VEYRA_VERSION=0.1.0 VEYRA_INSTALL_DIR=$HOME/bin sh install.sh
# The archive is verified against the release's SHA256SUMS before installing.
set -eu

REPO="iamzayn19/veyra"
INSTALL_DIR="${VEYRA_INSTALL_DIR:-$HOME/.local/bin}"

os=$(uname -s)
arch=$(uname -m)
case "$os-$arch" in
  Darwin-arm64) target=aarch64-apple-darwin ;;
  Darwin-x86_64) target=x86_64-apple-darwin ;;
  Linux-x86_64) target=x86_64-unknown-linux-gnu ;;
  Linux-aarch64 | Linux-arm64) target=aarch64-unknown-linux-gnu ;;
  *) echo "No prebuilt Veyra for $os/$arch. Build from source: https://github.com/$REPO" >&2; exit 1 ;;
esac

if [ -z "${VEYRA_VERSION:-}" ]; then
  VEYRA_VERSION=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | sed -n 's/.*"tag_name": *"v\{0,1\}\([^"]*\)".*/\1/p' | head -n1)
fi
[ -n "$VEYRA_VERSION" ] || { echo "Could not determine the latest version" >&2; exit 1; }

asset="veyra-v$VEYRA_VERSION-$target.tar.gz"
base="https://github.com/$REPO/releases/download/v$VEYRA_VERSION"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "Downloading $asset"
curl -fsSL -o "$tmp/$asset" "$base/$asset"
curl -fsSL -o "$tmp/SHA256SUMS" "$base/SHA256SUMS"

expected=$(grep " $asset\$" "$tmp/SHA256SUMS" | awk '{print $1}')
[ -n "$expected" ] || { echo "$asset is not listed in SHA256SUMS" >&2; exit 1; }
if command -v sha256sum >/dev/null 2>&1; then
  actual=$(sha256sum "$tmp/$asset" | awk '{print $1}')
else
  actual=$(shasum -a 256 "$tmp/$asset" | awk '{print $1}')
fi
[ "$expected" = "$actual" ] || { echo "Checksum mismatch for $asset" >&2; exit 1; }

tar -xzf "$tmp/$asset" -C "$tmp"
mkdir -p "$INSTALL_DIR"
install -m 755 "$(find "$tmp" -type f -name veyra | head -n1)" "$INSTALL_DIR/veyra"
echo "Installed veyra $VEYRA_VERSION to $INSTALL_DIR/veyra (sha256 verified)"
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) echo "Add $INSTALL_DIR to your PATH to run 'veyra'." ;;
esac
