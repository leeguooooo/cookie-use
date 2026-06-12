#!/bin/sh
# cookie-use installer — downloads the latest release binary (no npm, no token).
#   curl -fsSL https://raw.githubusercontent.com/leeguooooo/cookie-use/main/install.sh | sh
set -e

REPO="leeguooooo/cookie-use"
BIN="cookie-use"

os="$(uname -s)"
arch="$(uname -m)"

case "$os" in
  Darwin) ;;
  *) echo "cookie-use currently supports macOS only (got $os)." >&2; exit 1 ;;
esac

case "$arch" in
  arm64|aarch64) target="darwin-arm64" ;;
  x86_64|amd64)  target="darwin-x64" ;;
  *) echo "unsupported architecture: $arch" >&2; exit 1 ;;
esac

# Resolve the latest release tag.
tag="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
  | grep -m1 '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')"
if [ -z "$tag" ]; then
  echo "could not determine the latest cookie-use release." >&2
  exit 1
fi

url="https://github.com/${REPO}/releases/download/${tag}/${BIN}-${target}.tar.gz"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "downloading ${BIN} ${tag} (${target})..."
curl -fsSL "$url" -o "$tmp/${BIN}.tar.gz"
tar -xzf "$tmp/${BIN}.tar.gz" -C "$tmp"

dest="${HOME}/.local/bin"
mkdir -p "$dest"
install -m 0755 "$tmp/${BIN}" "$dest/${BIN}"

echo "installed ${BIN} -> ${dest}/${BIN}"
case ":$PATH:" in
  *":$dest:"*) ;;
  *) echo "note: add ${dest} to your PATH" ;;
esac

if ! command -v chrome-use >/dev/null 2>&1; then
  echo "note: cookie-use needs chrome-use. Install it with:" >&2
  echo "  curl -fsSL https://raw.githubusercontent.com/leeguooooo/chrome-use/main/install.sh | sh" >&2
fi
