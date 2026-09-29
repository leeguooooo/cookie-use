#!/bin/sh
# cookie-use installer — downloads a release binary (no npm, no token).
#   curl -fsSL https://raw.githubusercontent.com/leeguooooo/cookie-use/main/install.sh | sh
# Env: COOKIE_USE_BIN_DIR=<dir>     install there instead of ~/.local/bin
#                                   (`cookie-use upgrade` passes the running binary's directory)
#      COOKIE_USE_VERSION=vX.Y.Z    install that release instead of the latest
#                                   (`cookie-use upgrade --tag`); the new binary must report it
#
# Safety: the .sha256 sidecar is required and must match; the new binary must run
# (and report COOKIE_USE_VERSION when pinned) before it replaces anything; the swap
# is a rename in the destination directory, so a failure at any step leaves the
# existing cookie-use exactly as it was.
set -eu

REPO="leeguooooo/cookie-use"
BIN="cookie-use"

err() { echo "cookie-use install: $*" >&2; exit 1; }

os="$(uname -s)"
arch="$(uname -m)"

case "$os" in
  Darwin) ;;
  *) err "cookie-use currently supports macOS only (got $os)." ;;
esac

case "$arch" in
  arm64|aarch64) target="darwin-arm64" ;;
  x86_64|amd64)  target="darwin-x64" ;;
  *) err "unsupported architecture: $arch" ;;
esac

version="${COOKIE_USE_VERSION:-}"
case "$version" in
  "") ;;
  v[0-9]*.[0-9]*.[0-9]*) ;;
  [0-9]*.[0-9]*.[0-9]*) version="v$version" ;;
  *) err "COOKIE_USE_VERSION must look like v1.2.3 (got $version)" ;;
esac

# #6：不要走 api.github.com。未认证的 API 有速率限制，额度用尽就返回 403，安装直接失败
# ——而这跟本次安装该不该成功毫无关系。releases/latest/download/<asset> 是一条普通重定向，
# 不消耗 API 额度，也不需要任何 token。
# COOKIE_USE_DOWNLOAD_BASE is for tests (a file:// directory holding the assets).
if [ -n "${COOKIE_USE_DOWNLOAD_BASE:-}" ]; then
  base="$COOKIE_USE_DOWNLOAD_BASE"
elif [ -n "$version" ]; then
  base="https://github.com/${REPO}/releases/download/${version}"
else
  base="https://github.com/${REPO}/releases/latest/download"
fi
asset="${BIN}-${target}.tar.gz"
url="${base}/${asset}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "downloading ${BIN} ${version:-latest} (${target})..."
if ! curl -fsSL "$url" -o "$tmp/${BIN}.tar.gz"; then
  # 说清楚下不下来的是什么、以及人工怎么办——别只留一句 curl 的错误码。
  echo "could not download ${url}" >&2
  echo "check the releases page for an asset named ${asset}:" >&2
  echo "  https://github.com/${REPO}/releases/${version:+tag/$version}" >&2
  exit 1
fi

# The checksum is mandatory: a missing sidecar fails the install, it is never
# permission to skip verification.
curl -fsSL "${url}.sha256" -o "$tmp/${BIN}.tar.gz.sha256" \
  || err "could not download ${url}.sha256; refusing to install an unverified binary"
expected="$(awk '{print $1; exit}' "$tmp/${BIN}.tar.gz.sha256")"
[ "${#expected}" -eq 64 ] || err "invalid checksum file ${url}.sha256"
case "$expected" in *[!0-9a-fA-F]*) err "invalid checksum file ${url}.sha256" ;; esac
actual="$(shasum -a 256 "$tmp/${BIN}.tar.gz" | awk '{print $1}')"
if [ "$expected" != "$actual" ]; then
  echo "checksum mismatch for ${asset}" >&2
  echo "  expected $expected" >&2
  echo "  actual   $actual" >&2
  exit 1
fi
echo "checksum ok"

tar -xzf "$tmp/${BIN}.tar.gz" -C "$tmp"
[ -f "$tmp/${BIN}" ] && [ ! -L "$tmp/${BIN}" ] || err "archive did not contain a ${BIN} binary"
chmod 0755 "$tmp/${BIN}"
got="$("$tmp/${BIN}" --version 2>/dev/null)" || err "the downloaded ${BIN} does not run; nothing was replaced"
if [ -n "$version" ] && [ "${got##* }" != "${version#v}" ]; then
  err "the downloaded binary reports '${got}', expected ${version#v}; nothing was replaced"
fi

dest="${COOKIE_USE_BIN_DIR:-${HOME}/.local/bin}"
mkdir -p "$dest"
# Stage next to the target, then rename over it: the swap is atomic, a failed
# copy never leaves a half-written binary, and the running binary's inode is
# left untouched (overwriting it in place invalidates its code signature).
install -m 0755 "$tmp/${BIN}" "$dest/.${BIN}.new.$$" || { rm -f "$dest/.${BIN}.new.$$"; err "could not stage ${dest}/${BIN}"; }
if ! mv -f "$dest/.${BIN}.new.$$" "$dest/${BIN}"; then
  rm -f "$dest/.${BIN}.new.$$"
  err "could not replace ${dest}/${BIN}"
fi

echo "installed ${got} -> ${dest}/${BIN}"
case ":$PATH:" in
  *":$dest:"*) ;;
  *) echo "note: add ${dest} to your PATH" ;;
esac

if ! command -v chrome-use >/dev/null 2>&1; then
  echo "note: cookie-use needs chrome-use. Install it with:" >&2
  echo "  curl -fsSL https://raw.githubusercontent.com/leeguooooo/chrome-use/main/install.sh | sh" >&2
fi
