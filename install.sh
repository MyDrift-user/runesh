#!/usr/bin/env bash
# RUNESH CLI installer for Linux / macOS
# Usage: curl -fsSL https://raw.githubusercontent.com/mydrift-user/runesh/main/install.sh | bash
set -euo pipefail

REPO="${RUNESH_REPO:-mydrift-user/runesh}"
BIN_NAME="runesh"
INSTALL_DIR="${RUNESH_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf "\033[1;36m[runesh]\033[0m %s\n" "$*"; }
err() { printf "\033[1;31m[runesh] error:\033[0m %s\n" "$*" >&2; exit 1; }

command -v curl >/dev/null || err "curl is required"
command -v tar  >/dev/null || err "tar is required"

os="$(uname -s)"
arch="$(uname -m)"
case "$os-$arch" in
  Linux-x86_64)   target="x86_64-unknown-linux-gnu" ;;
  Linux-aarch64)  target="aarch64-unknown-linux-gnu" ;;
  Darwin-x86_64)  target="x86_64-apple-darwin" ;;
  Darwin-arm64)   target="aarch64-apple-darwin" ;;
  *) err "unsupported platform: $os-$arch" ;;
esac

version="${RUNESH_VERSION:-latest}"
if [ "$version" = "latest" ]; then
  tag=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | grep -oE '"tag_name": *"[^"]+"' | head -n1 | cut -d'"' -f4)
  [ -n "$tag" ] || err "could not resolve latest release"
else
  tag="$version"
fi

asset="${BIN_NAME}-${target}.tar.gz"
url="https://github.com/$REPO/releases/download/$tag/$asset"

say "downloading $asset ($tag)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
curl -fsSL "$url" -o "$tmp/$asset" || err "download failed: $url"

tar -xzf "$tmp/$asset" -C "$tmp"
mkdir -p "$INSTALL_DIR"
install -m 0755 "$tmp/${BIN_NAME}-${target}/${BIN_NAME}" "$INSTALL_DIR/${BIN_NAME}"

say "installed $INSTALL_DIR/$BIN_NAME"
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) say "add to PATH:  export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
esac

"$INSTALL_DIR/$BIN_NAME" --version || true
