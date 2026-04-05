#!/bin/sh
set -e

REPO="andreasasprou/aps"
INSTALL_DIR="${APS_INSTALL_DIR:-/usr/local/bin}"

# Detect OS and architecture
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$OS" in
  darwin) PLATFORM="darwin" ;;
  linux)  PLATFORM="linux" ;;
  *)
    echo "Error: Unsupported OS: $OS"
    echo "Install manually: cargo install --git https://github.com/$REPO"
    exit 1
    ;;
esac

case "$ARCH" in
  x86_64|amd64)  ARCH="x86_64" ;;
  arm64|aarch64) ARCH="arm64" ;;
  *)
    echo "Error: Unsupported architecture: $ARCH"
    echo "Install manually: cargo install --git https://github.com/$REPO"
    exit 1
    ;;
esac

BINARY="aps-${PLATFORM}-${ARCH}"

# Get latest release tag
echo "Finding latest release..."
LATEST=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | grep '"tag_name"' | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')

if [ -z "$LATEST" ]; then
  echo "Error: Could not find latest release."
  echo "Install manually: cargo install --git https://github.com/$REPO"
  exit 1
fi

URL="https://github.com/$REPO/releases/download/${LATEST}/${BINARY}.tar.gz"

echo "Downloading aps $LATEST for $PLATFORM/$ARCH..."
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

curl -fsSL "$URL" -o "$TMPDIR/aps.tar.gz"
tar xzf "$TMPDIR/aps.tar.gz" -C "$TMPDIR"

# Install binary
if [ -w "$INSTALL_DIR" ]; then
  mv "$TMPDIR/aps" "$INSTALL_DIR/aps"
else
  echo "Installing to $INSTALL_DIR (requires sudo)..."
  sudo mv "$TMPDIR/aps" "$INSTALL_DIR/aps"
fi

chmod +x "$INSTALL_DIR/aps"

echo ""
echo "✓ aps $LATEST installed to $INSTALL_DIR/aps"
echo ""
echo "Get started:"
echo "  aps auth claude --label myaccount    # Authenticate Claude account"
echo "  aps auth codex --label myaccount     # Authenticate Codex account"
echo "  aps status --all                     # See all usage"
