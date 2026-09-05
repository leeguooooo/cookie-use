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

# #6：不要走 api.github.com。未认证的 API 有速率限制，额度用尽就返回 403，安装直接失败
# ——而这跟本次安装该不该成功毫无关系。releases/latest/download/<asset> 是一条普通重定向，
# 不消耗 API 额度，也不需要任何 token。
url="https://github.com/${REPO}/releases/latest/download/${BIN}-${target}.tar.gz"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "downloading the latest ${BIN} (${target})..."
if ! curl -fsSL "$url" -o "$tmp/${BIN}.tar.gz"; then
  # 说清楚下不下来的是什么、以及人工怎么办——别只留一句 curl 的错误码。
  echo "could not download ${url}" >&2
  echo "check the releases page for an asset named ${BIN}-${target}.tar.gz:" >&2
  echo "  https://github.com/${REPO}/releases/latest" >&2
  exit 1
fi
tar -xzf "$tmp/${BIN}.tar.gz" -C "$tmp"

# 校验和是发布的（sha256 sidecar），能拿到就核；拿不到不阻断安装。
if curl -fsSL "${url}.sha256" -o "$tmp/${BIN}.tar.gz.sha256" 2>/dev/null; then
  expected="$(awk '{print $1; exit}' "$tmp/${BIN}.tar.gz.sha256")"
  actual="$(shasum -a 256 "$tmp/${BIN}.tar.gz" | awk '{print $1}')"
  if [ "$expected" != "$actual" ]; then
    echo "checksum mismatch for ${BIN}-${target}.tar.gz" >&2
    echo "  expected $expected" >&2
    echo "  actual   $actual" >&2
    exit 1
  fi
  echo "checksum ok"
fi

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
