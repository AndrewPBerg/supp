#!/usr/bin/env bash
set -euo pipefail

REPO="AndrewPBerg/supp"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"

# Detect OS and architecture
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Linux)  os="unknown-linux-gnu" ;;
  Darwin) os="apple-darwin" ;;
  *)      echo "Unsupported OS: $OS (try the Windows zip from GitHub Releases)"; exit 1 ;;
esac

# Use musl on Alpine / non-glibc systems
if [ "$OS" = "Linux" ] && ! ldd --version 2>&1 | grep -qi glibc; then
  os="unknown-linux-musl"
fi

case "$ARCH" in
  x86_64|amd64)  arch="x86_64" ;;
  aarch64|arm64) arch="aarch64" ;;
  *)             echo "Unsupported architecture: $ARCH"; exit 1 ;;
esac

# musl builds are only available for x86_64
if [ "$os" = "unknown-linux-musl" ] && [ "$arch" != "x86_64" ]; then
  echo "No musl build available for $arch — try installing from source with: cargo install supp"
  exit 1
fi

TARGET="${arch}-${os}"

# Get latest version if not specified
if [ -z "${VERSION:-}" ]; then
  VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed 's/.*"v\(.*\)".*/\1/')"
fi

echo "Installing supp v${VERSION} (${TARGET})..."

TARBALL="supp-${TARGET}.tar.gz"
URL="https://github.com/${REPO}/releases/download/v${VERSION}/${TARBALL}"
CHECKSUM_URL="https://github.com/${REPO}/releases/download/v${VERSION}/SHA256SUMS"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

curl -fsSL "$URL" -o "${TMP}/${TARBALL}"
curl -fsSL "$CHECKSUM_URL" -o "${TMP}/SHA256SUMS"

# Verify checksum
cd "$TMP" && grep "$TARBALL" SHA256SUMS | sha256sum -c -

tar xzf "${TMP}/${TARBALL}" -C "$TMP"

if [ -w "$INSTALL_DIR" ]; then
  mv "${TMP}/supp" "${INSTALL_DIR}/supp"
else
  sudo mv "${TMP}/supp" "${INSTALL_DIR}/supp"
fi

echo "supp v${VERSION} installed to ${INSTALL_DIR}/supp"
