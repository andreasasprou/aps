#!/bin/sh
set -e

REPO="andreasasprou/aps"
INSTALL_DIR="${APS_INSTALL_DIR:-$HOME/.local/bin}"

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
mkdir -p "$INSTALL_DIR"
mv "$TMPDIR/aps" "$INSTALL_DIR/aps"
chmod +x "$INSTALL_DIR/aps"

echo ""
echo "✓ aps $LATEST installed to $INSTALL_DIR/aps"

# Add to PATH in shell RC if not already there
SHELL_RC=""
case "${SHELL:-}" in
  */zsh)  SHELL_RC="$HOME/.zshrc" ;;
  */bash) SHELL_RC="$HOME/.bashrc" ;;
esac

# Fallback: check which RC file exists
if [ -z "$SHELL_RC" ]; then
  if [ -f "$HOME/.zshrc" ]; then
    SHELL_RC="$HOME/.zshrc"
  elif [ -f "$HOME/.bashrc" ]; then
    SHELL_RC="$HOME/.bashrc"
  fi
fi

PATH_LINE="export PATH=\"$INSTALL_DIR:\$PATH\""

case ":$PATH:" in
  *":$INSTALL_DIR:"*)
    # Already in PATH, nothing to do
    ;;
  *)
    if [ -n "$SHELL_RC" ]; then
      # Check if already added to RC file
      if ! grep -qF "$INSTALL_DIR" "$SHELL_RC" 2>/dev/null; then
        echo "" >> "$SHELL_RC"
        echo "# aps - agent profile switcher" >> "$SHELL_RC"
        echo "$PATH_LINE" >> "$SHELL_RC"
        echo "✓ Added $INSTALL_DIR to PATH in $SHELL_RC"
      fi
      # Source it so it works immediately in this session
      export PATH="$INSTALL_DIR:$PATH"
    else
      echo ""
      echo "Add to your PATH manually:"
      echo "  $PATH_LINE"
    fi
    ;;
esac

echo ""
echo "Get started:"
echo "  aps auth claude --label myaccount    # Authenticate Claude account"
echo "  aps auth codex --label myaccount     # Authenticate Codex account"
echo "  aps status --all                     # See all usage"
