#!/bin/sh
# kern one-line installer (Linux / macOS).
#
#   curl -fsSL https://raw.githubusercontent.com/yesitsfebreeze/relay-kern/master/install.sh | sh
#
# Downloads the prebuilt `kern` binary for this platform from the latest GitHub
# release and installs it to ~/.local/bin (override with KERN_BIN_DIR).
set -eu

REPO="yesitsfebreeze/relay-kern"
BIN_DIR="${KERN_BIN_DIR:-$HOME/.local/bin}"

os="$(uname -s)"
arch="$(uname -m)"

case "$os" in
  Linux)
    case "$arch" in
      x86_64|amd64) target="x86_64-unknown-linux-gnu" ;;
      *) echo "kern: no prebuilt binary for Linux/$arch (build from source)"; exit 1 ;;
    esac ;;
  Darwin)
    case "$arch" in
      arm64|aarch64) target="aarch64-apple-darwin" ;;
      x86_64) target="x86_64-apple-darwin" ;;
      *) echo "kern: no prebuilt binary for macOS/$arch"; exit 1 ;;
    esac ;;
  *)
    echo "kern: unsupported OS '$os' — use the Windows installer (install.ps1) or build from source"
    exit 1 ;;
esac

url="https://github.com/$REPO/releases/latest/download/kern-$target.tar.gz"
echo "kern: downloading $target ..."
mkdir -p "$BIN_DIR"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

if ! curl -fSL "$url" -o "$tmp/kern.tar.gz"; then
  echo "kern: download failed ($url)"
  echo "      no release yet? see https://github.com/$REPO/releases"
  exit 1
fi
tar xzf "$tmp/kern.tar.gz" -C "$tmp"
mv "$tmp/kern" "$BIN_DIR/kern"
chmod +x "$BIN_DIR/kern"

echo "kern: installed to $BIN_DIR/kern"
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) echo "kern: add $BIN_DIR to your PATH, e.g.  export PATH=\"$BIN_DIR:\$PATH\"" ;;
esac
echo "kern: next — register the MCP server:  claude mcp add kern -- $BIN_DIR/kern mcp"
